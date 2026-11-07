mod common;

use common::require_safe_test_db_url;
use spine_api::db::migrations::{
    run_migrations, run_migrations_with_pause, SPINE_MIGRATION_ADVISORY_LOCK_ID,
};
use sqlx::Row;
use std::sync::Arc;
use tokio::sync::Notify;

#[tokio::test]
async fn test_actual_run_migrations_cancellation_and_recovery() {
    let url = require_safe_test_db_url();

    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test database");

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

    let pause = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());

    let pause_clone = pause.clone();
    let resume_clone = resume.clone();

    let (tx_abort, mut rx_abort) = tokio::sync::oneshot::channel::<()>();
    let pool_a_task = pool_a.clone();
    let task_thread = std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        rt.block_on(async move {
            tokio::select! {
                res = run_migrations_with_pause(pool_a_task, Some(pause_clone), Some(resume_clone)) => res,
                _ = &mut rx_abort => Err(spine_api::core::error::ForgeError::Internal("aborted".into())),
            }
        })
    });

    // Wait until Task A has entered its transaction and actively acquired the advisory lock
    pause.notified().await;

    // 2. Verify from separate connection that Worker A's transaction-level advisory lock is actively held in pg_locks
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
        "Transaction-level advisory lock must be actively held in pg_locks by actual run_migrations"
    );

    // 3. Verify that Worker B cannot acquire the advisory lock concurrently
    let mut tx_b = pool_b.begin().await.expect("begin tx_b");
    let try_lock: (bool,) = sqlx::query_as("SELECT pg_try_advisory_xact_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .fetch_one(&mut *tx_b)
        .await
        .expect("try pg_try_advisory_xact_lock");

    assert!(
        !try_lock.0,
        "Worker B must fail to acquire advisory lock while actual run_migrations in Task A holds it"
    );
    tx_b.rollback().await.expect("rollback tx_b");

    // 4. Cancel/abort the running task executing run_migrations
    let _ = tx_abort.send(());
    let thread_res = task_thread.join().expect("join worker A thread");
    assert!(
        thread_res.is_err(),
        "Task A must have been aborted/cancelled"
    );

    // Close Worker A pool connection so PostgreSQL terminates the backend session and cleans up transaction
    pool_a.close().await;

    // Small yield to let PostgreSQL engine finish rolling back the aborted transaction
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 5. Verify transaction-level advisory lock was instantly and cleanly released upon task abortion
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

    // 7. Verify all 12 migrations are applied and committed
    let final_migrations: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count completed migrations");

    assert_eq!(
        final_migrations.0, 12,
        "Worker B must have committed all 12 migrations"
    );

    // 8. Verify idempotent re-run
    run_migrations(&pool_b)
        .await
        .expect("Subsequent migration run must be idempotent");
}
