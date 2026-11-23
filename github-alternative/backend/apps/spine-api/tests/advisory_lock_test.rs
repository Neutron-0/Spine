mod common;

use common::HarnessDisposableDb;
use spine_api::db::migrations::{
    run_migrations, run_migrations_with_pause, run_migrations_with_pause_after_step,
    SPINE_MIGRATION_ADVISORY_LOCK_ID,
};
use sqlx::Row;
use std::sync::Arc;
use tokio::sync::Notify;

#[tokio::test]
async fn test_actual_run_migrations_cancellation_and_recovery() {
    if std::env::var("SPINE_TEST_DATABASE_URL").is_err() {
        eprintln!("Skipping advisory lock test: SPINE_TEST_DATABASE_URL not set");
        return;
    }

    let disp_db = HarnessDisposableDb::provision()
        .await
        .expect("provision disposable database");
    let url = disp_db.migrator_url.clone();

    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test database");

    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name != '_harness_provision_proof'"
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
          AND (database = 0 OR database = (SELECT oid FROM pg_database WHERE datname = current_database()))
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
          AND (database = 0 OR database = (SELECT oid FROM pg_database WHERE datname = current_database()))
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

    // 7. Verify all 16 migrations are applied and committed
    let final_migrations: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count completed migrations");

    assert_eq!(
        final_migrations.0, 16,
        "Worker B must have committed all 16 migrations"
    );

    // 8. Verify idempotent re-run
    run_migrations(&pool_b)
        .await
        .expect("Subsequent migration run must be idempotent");

    pool.close().await;
    pool_b.close().await;
    disp_db
        .cleanup()
        .await
        .expect("cleanup disposable database");
}

#[tokio::test]
async fn test_repeated_migration_cancellation_during_real_ddl_and_inserts() {
    if std::env::var("SPINE_TEST_DATABASE_URL").is_err() {
        eprintln!("Skipping repeated advisory lock test: SPINE_TEST_DATABASE_URL not set");
        return;
    }

    let disp_db = HarnessDisposableDb::provision()
        .await
        .expect("provision disposable database");
    let url = disp_db.migrator_url.clone();
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test db");

    // Repeat the transactional cancellation and recovery workflow 5 times
    for cycle in 1..=5 {
        println!(
            "Starting transactional migration cancellation cycle {} of 5...",
            cycle
        );

        // Clean existing tables (preserving proof table)
        let tables: Vec<(String,)> = sqlx::query_as(
            "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name != '_harness_provision_proof'"
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

        // Worker A executes real DDL and pauses mid-migration (after step 0: 0001_core_schema)
        let task_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            rt.block_on(async move {
                tokio::select! {
                    res = run_migrations_with_pause_after_step(pool_a_task, 0, Some(pause_clone), Some(resume_clone)) => res,
                    _ = &mut rx_abort => Err(spine_api::core::error::ForgeError::Internal("aborted".into())),
                }
            })
        });

        // Wait until Worker A has applied step 0 DDL and is paused
        pause.notified().await;

        // Worker B attempts migration concurrently; it must block on Worker A's transaction advisory lock
        let pool_b_task = pool_b.clone();
        let (done_b_tx, mut done_b_rx) =
            tokio::sync::oneshot::channel::<Result<(), spine_api::core::error::ForgeError>>();
        let worker_b_thread = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let res = rt.block_on(async move { run_migrations(&pool_b_task).await });
            let _ = done_b_tx.send(res);
        });

        // Assert Worker B is blocked waiting for advisory lock
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(
            done_b_rx.try_recv().is_err(),
            "Worker B must be blocked waiting on advisory lock"
        );

        // Cancel Worker A: abort the task and close connection pool
        let _ = tx_abort.send(());
        let _ = task_thread.join().expect("join worker A");
        pool_a.close().await;

        // Worker B unblocks automatically and completes all migrations
        let worker_b_res = tokio::time::timeout(std::time::Duration::from_secs(15), &mut done_b_rx)
            .await
            .expect("Worker B should complete within timeout")
            .expect("Worker B channel should receive result");

        assert!(
            worker_b_res.is_ok(),
            "Worker B must succeed after Worker A cancellation: {:?}",
            worker_b_res
        );
        worker_b_thread.join().expect("join worker B thread");

        // Verify all 16 migrations were cleanly committed
        let final_count: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
            .fetch_one(&pool)
            .await
            .expect("count completed migrations");
        assert_eq!(
            final_count.0, 16,
            "Cycle {}: All 16 migrations must be committed",
            cycle
        );

        pool_b.close().await;
        println!(
            "Transactional migration cancellation cycle {} of 5 PASSED cleanly.",
            cycle
        );
    }

    pool.close().await;
    disp_db
        .cleanup()
        .await
        .expect("cleanup disposable database");
}
