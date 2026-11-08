mod common;

use common::HarnessDisposableDb;
use spine_api::backup::{ConsistencyBarrier, SPINE_CLUSTER_MAINTENANCE_LOCK_ID};
use sqlx::Row;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

async fn run_child_writer(url: &str) {
    let pool = sqlx::PgPool::connect(url).await.expect("child connect");
    let barrier = ConsistencyBarrier::with_pool(pool);
    match barrier.acquire_write_permit().await {
        Ok(permit) => {
            eprintln!("RESULT:ACQUIRED");
            let _ = permit.release().await;
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("RESULT:DENIED: {}", e);
            std::process::exit(2);
        }
    }
}

async fn run_child_holder(url: &str) {
    let pool = sqlx::PgPool::connect(url).await.expect("child connect");
    let barrier = ConsistencyBarrier::with_pool(pool);
    match barrier.acquire_write_permit().await {
        Ok(_permit) => {
            eprintln!("READY:HELD");
            // Hold permit indefinitely until killed by parent
            tokio::time::sleep(Duration::from_secs(300)).await;
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("ERROR:HELD_FAILED: {}", e);
            std::process::exit(1);
        }
    }
}

async fn run_child_maintenance(url: &str) {
    let pool = sqlx::PgPool::connect(url).await.expect("child connect");
    let barrier = ConsistencyBarrier::with_pool(pool);
    match barrier.enter_maintenance(Duration::from_millis(500)).await {
        Ok(guard) => {
            eprintln!("RESULT:MAINT_ACQUIRED");
            let _ = guard.release().await;
            std::process::exit(0);
        }
        Err(e) => {
            eprintln!("RESULT:MAINT_DENIED: {}", e);
            std::process::exit(3);
        }
    }
}

#[tokio::test]
async fn test_cross_process_barrier_coordination() {
    let raw_url = match std::env::var("SPINE_TEST_DATABASE_URL") {
        Ok(u) if !u.trim().is_empty() => u,
        _ => {
            eprintln!("Skipping cross-process coordination test: SPINE_TEST_DATABASE_URL not set");
            return;
        }
    };

    // Check if this execution is a child process invocation
    if std::env::var("SPINE_CHILD_MODE").as_deref() == Ok("writer") {
        run_child_writer(&raw_url).await;
        return;
    }
    if std::env::var("SPINE_CHILD_MODE").as_deref() == Ok("holder") {
        run_child_holder(&raw_url).await;
        return;
    }
    if std::env::var("SPINE_CHILD_MODE").as_deref() == Ok("maintenance") {
        run_child_maintenance(&raw_url).await;
        return;
    }

    // --- PARENT PROCESS TEST LOGIC ---
    let disp_db = HarnessDisposableDb::provision()
        .await
        .expect("provision ephemeral test database");
    let url = disp_db.app_url.clone();

    let pool = sqlx::PgPool::connect(&url).await.expect("connect parent");
    let current_exe = std::env::current_exe().expect("current_exe");
    let barrier = ConsistencyBarrier::with_pool(pool.clone());

    println!("Parent Process: Entering exclusive cluster maintenance...");
    let guard = barrier
        .enter_maintenance(Duration::from_secs(10))
        .await
        .expect("enter maintenance");

    // 1. Independent OS Process B attempts writer permit while Parent holds maintenance
    println!(
        "Spawning Child Process B to attempt write permit while Parent holds maintenance guard..."
    );
    let child_output = Command::new(&current_exe)
        .arg("test_cross_process_barrier_coordination")
        .arg("--exact")
        .arg("--nocapture")
        .env("SPINE_CHILD_MODE", "writer")
        .env("SPINE_TEST_DATABASE_URL", &url)
        .env("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1")
        .output()
        .expect("spawn child writer");

    let stderr_str = String::from_utf8_lossy(&child_output.stderr);
    println!("Child Process B stderr: {}", stderr_str);
    assert_eq!(
        child_output.status.code(),
        Some(2),
        "Child Process B must exit with code 2 (denied) while Parent holds maintenance"
    );
    assert!(
        stderr_str.contains("RESULT:DENIED"),
        "Child Process B must output RESULT:DENIED"
    );

    // 2. Child Process D attempts second maintenance while Parent holds maintenance -> MUST FAIL
    println!(
        "Spawning Child Process D to attempt simultaneous maintenance while Parent holds guard..."
    );
    let child_maint_output = Command::new(&current_exe)
        .arg("test_cross_process_barrier_coordination")
        .arg("--exact")
        .arg("--nocapture")
        .env("SPINE_CHILD_MODE", "maintenance")
        .env("SPINE_TEST_DATABASE_URL", &url)
        .env("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1")
        .output()
        .expect("spawn child maintenance");

    let maint_stderr_str = String::from_utf8_lossy(&child_maint_output.stderr);
    println!("Child Process D stderr: {}", maint_stderr_str);
    assert_eq!(
        child_maint_output.status.code(),
        Some(3),
        "Two maintenance guards must not be active simultaneously"
    );
    assert!(
        maint_stderr_str.contains("RESULT:MAINT_DENIED"),
        "Child Process D must report maintenance denied"
    );

    // 3. Parent releases MaintenanceGuard explicitly
    println!("Parent Process: Explicitly releasing maintenance guard...");
    guard.release().await.expect("explicit release guard");
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 4. Independent OS Process B attempts writer permit again; must be admitted
    println!("Spawning Child Process B again after maintenance released...");
    let child_output_after = Command::new(&current_exe)
        .arg("test_cross_process_barrier_coordination")
        .arg("--exact")
        .arg("--nocapture")
        .env("SPINE_CHILD_MODE", "writer")
        .env("SPINE_TEST_DATABASE_URL", &url)
        .env("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1")
        .output()
        .expect("spawn child writer after");

    let stderr_after_str = String::from_utf8_lossy(&child_output_after.stderr);
    println!("Child Process B second stderr: {}", stderr_after_str);
    assert_eq!(
        child_output_after.status.code(),
        Some(0),
        "Child Process B must exit with code 0 (acquired) after Parent releases maintenance"
    );
    assert!(
        stderr_after_str.contains("RESULT:ACQUIRED"),
        "Child Process B must output RESULT:ACQUIRED"
    );

    // 5. Test writer admitted before maintenance causing maintenance to wait/timeout
    println!("Spawning Child Process C with persistent write permit...");
    let mut child_holder = Command::new(&current_exe)
        .arg("test_cross_process_barrier_coordination")
        .arg("--exact")
        .arg("--nocapture")
        .env("SPINE_CHILD_MODE", "holder")
        .env("SPINE_TEST_DATABASE_URL", &url)
        .env("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1")
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn child holder");

    let stderr = child_holder.stderr.take().expect("take stderr");
    let mut reader = BufReader::new(stderr);
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .expect("read line from holder");
    println!("Child Process C announced: {}", first_line.trim());
    assert!(first_line.contains("READY:HELD"));

    // Verify shared advisory lock is held in pg_locks
    let lock_row = sqlx::query(
        r#"
        SELECT count(*) as cnt 
        FROM pg_locks 
        WHERE locktype = 'advisory' 
          AND classid = (($1::bigint >> 32) & 4294967295)::integer
          AND objid = ($1::bigint & 4294967295)::integer
          AND mode = 'ExclusiveLock'
        "#,
    )
    .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
    .fetch_one(&pool)
    .await
    .expect("query pg_locks");

    let excl_count: i64 = lock_row.get("cnt");
    assert_eq!(
        excl_count, 0,
        "Exclusive lock must not be held while child writer holds shared permit"
    );

    // Parent attempts short maintenance window; must time out because Child C holds permit
    println!("Parent Process: Attempting short maintenance while Child C holds permit (must time out)...");
    let maint_attempt = barrier.enter_maintenance(Duration::from_millis(300)).await;
    assert!(
        maint_attempt.is_err(),
        "Maintenance must time out while active writer holds shared advisory lock"
    );

    // 6. Test uncooperative OS process kill cleanup
    println!("Killing uncooperative Child Process C...");
    child_holder.kill().expect("kill child holder");
    let _ = child_holder.wait();

    // Give PostgreSQL engine a moment to tear down connection and release session locks
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 7. Verify Parent can immediately acquire exclusive maintenance lock without blocking
    println!("Parent Process: Verifying exclusive maintenance acquisition succeeds cleanly...");
    let guard_after_kill = barrier
        .enter_maintenance(Duration::from_secs(5))
        .await
        .expect("acquire maintenance after child kill");
    println!("Parent Process: Exclusive maintenance guard acquired successfully after uncooperative process kill.");
    guard_after_kill
        .release()
        .await
        .expect("release guard after kill");

    drop(barrier);
    pool.close().await;

    println!("Cleaning up disposable test database...");
    disp_db
        .cleanup()
        .await
        .expect("cleanup disposable database");

    println!("All Cross-Process Barrier Coordination tests PASSED.");
}
