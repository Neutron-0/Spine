use spine_api::db::migrations::SPINE_MIGRATION_ADVISORY_LOCK_ID;
use sqlx::Row;

fn get_db_url() -> String {
    std::env::var("TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .unwrap_or_else(|_| {
            "postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge?sslmode=disable"
                .to_string()
        })
}

#[tokio::test]
async fn test_migration_advisory_xact_lock_lifecycle_and_cancellation_safety() {
    let url = get_db_url();
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

    // 1. Task 1 begins a transaction and acquires the transaction-level advisory lock
    let mut tx1 = pool.begin().await.expect("begin tx1");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .execute(&mut *tx1)
        .await
        .expect("acquire advisory xact lock in tx1");

    // Verify lock is present in pg_locks
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
    .expect("query pg_locks");

    let count: i64 = lock_row.get("cnt");
    assert!(
        count >= 1,
        "Transaction-level advisory lock must be visible in pg_locks"
    );

    // 2. From a separate connection (tx2), verify try_advisory_xact_lock fails (returns false)
    let mut tx2 = pool.begin().await.expect("begin tx2");
    let try_row = sqlx::query("SELECT pg_try_advisory_xact_lock($1) as acquired")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .fetch_one(&mut *tx2)
        .await
        .expect("try lock in tx2");
    let acquired: bool = try_row.get("acquired");
    assert!(
        !acquired,
        "Concurrent transaction must NOT be able to acquire held migration lock"
    );
    tx2.rollback().await.expect("rollback tx2");

    // 3. Simulate cancellation or abort of Task 1 (drop tx1 without committing)
    drop(tx1);

    // Small yield to allow PostgreSQL backend process to clean up the rolled-back connection
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // 4. Verify lock was automatically freed by the engine upon transaction termination
    let mut tx3 = pool.begin().await.expect("begin tx3");
    let try_row3 = sqlx::query("SELECT pg_try_advisory_xact_lock($1) as acquired")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .fetch_one(&mut *tx3)
        .await
        .expect("try lock in tx3 after tx1 aborted");
    let acquired3: bool = try_row3.get("acquired");
    assert!(
        acquired3,
        "Lock must be instantly available to next transaction after task abort/drop"
    );

    // Commit tx3 and verify lock is cleanly released
    tx3.commit().await.expect("commit tx3");

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
    .expect("query pg_locks after commit");

    let final_count: i64 = final_lock_row.get("cnt");
    assert_eq!(
        final_count, 0,
        "Transaction-level advisory lock must be completely absent from pg_locks after commit"
    );
}
