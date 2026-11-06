mod common;

use common::get_safe_test_db_url;
use spine_api::db::migrations::{run_migrations, SPINE_MIGRATION_ADVISORY_LOCK_ID};
use sqlx::Row;

#[tokio::test]
async fn test_actual_run_migrations_cancellation_and_recovery() {
    let url = match get_safe_test_db_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIPPED: test requires SPINE_TEST_DATABASE_URL to be set.");
            return;
        }
    };

    let pool = match sqlx::PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Skipping advisory lock test: PostgreSQL unreachable ({})",
                e
            );
            return;
        }
    };

    // Clean existing tables in the disposable test database
    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE'"
    )
    .fetch_all(&pool)
    .await
    .expect("fetch tables");

    for (tbl,) in tables {
        sqlx::query(&format!("DROP TABLE IF EXISTS \"{}\" CASCADE", tbl))
            .execute(&pool)
            .await
            .expect("drop table");
    }

    let pool_a = sqlx::PgPool::connect(&url).await.expect("connect worker A");
    let pool_b = sqlx::PgPool::connect(&url).await.expect("connect worker B");

    // 1. Worker A acquires transaction-level advisory lock
    let mut tx_a = pool_a.begin().await.expect("begin tx_a");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .execute(&mut *tx_a)
        .await
        .expect("acquire pg_advisory_xact_lock");

    // 2. Verify from separate connection that Worker A's transaction-level advisory lock is active
    let lock_row = sqlx::query(
        r#"
        SELECT count(*) as cnt 
        FROM pg_locks 
        WHERE locktype = 'advisory' 
          AND classid = (($1::bigint >> 32) & 4294967295)::integer
          AND objid = ($1::bigint & 4294967295)::integer
        "#,
    )
    .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
    .fetch_one(&pool)
    .await
    .expect("query pg_locks during worker A lock");

    let count: i64 = lock_row.get("cnt");
    assert_eq!(
        count, 1,
        "Transaction-level advisory lock must be actively held in pg_locks"
    );

    // 3. Verify that Worker B cannot acquire the lock concurrently
    let mut tx_b = pool_b.begin().await.expect("begin tx_b");
    let try_lock: (bool,) = sqlx::query_as("SELECT pg_try_advisory_xact_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .fetch_one(&mut *tx_b)
        .await
        .expect("try pg_try_advisory_xact_lock");

    assert!(
        !try_lock.0,
        "Worker B must fail to acquire advisory lock while Worker A holds it"
    );
    tx_b.rollback().await.expect("rollback tx_b");

    // 4. Abort Worker A task/transaction (simulating crash or connection drop)
    drop(tx_a);

    // Small yield to let PostgreSQL engine finish rolling back the dropped transaction
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 5. Verify transaction-level advisory lock was instantly released upon drop
    let final_lock_row = sqlx::query(
        r#"
        SELECT count(*) as cnt 
        FROM pg_locks 
        WHERE locktype = 'advisory' 
          AND classid = (($1::bigint >> 32) & 4294967295)::integer
          AND objid = ($1::bigint & 4294967295)::integer
        "#,
    )
    .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
    .fetch_one(&pool)
    .await
    .expect("query pg_locks after abort");

    let final_count: i64 = final_lock_row.get("cnt");
    assert_eq!(
        final_count, 0,
        "Transaction-level advisory lock must be completely absent from pg_locks after abort"
    );

    // 6. Worker B connects and successfully runs migrations to completion
    run_migrations(&pool_b)
        .await
        .expect("Worker B must acquire lock and complete all migrations after Worker A abort");

    // 7. Verify all 11 migrations are applied and committed
    let final_migrations: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count completed migrations");

    assert_eq!(
        final_migrations.0, 11,
        "Worker B must have committed all 11 migrations"
    );

    // 8. Verify idempotent re-run
    run_migrations(&pool_b)
        .await
        .expect("Subsequent migration run must be idempotent");
}
