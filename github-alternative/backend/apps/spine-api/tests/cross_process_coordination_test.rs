mod common;

use common::{ensure_disposable_marker, require_safe_test_db_url, verify_disposable_database};
use spine_api::backup::{ConsistencyBarrier, SPINE_CLUSTER_MAINTENANCE_LOCK_ID};
use sqlx::Row;
use std::io::{BufRead, BufReader};
use std::process::{Command, Stdio};
use std::time::Duration;

async fn run_child_writer(url: &str) {
    let pool = sqlx::PgPool::connect(url).await.expect("child connect");
    let barrier = ConsistencyBarrier::with_pool(pool);
    match barrier.acquire_write_permit().await {
        Ok(_permit) => {
            eprintln!("RESULT:ACQUIRED");
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

#[tokio::test]
async fn test_cross_process_barrier_coordination() {
    let url = require_safe_test_db_url();

    // Check if this execution is a child process invocation
    if std::env::var("SPINE_CHILD_MODE").as_deref() == Ok("writer") {
        run_child_writer(&url).await;
        return;
    }
    if std::env::var("SPINE_CHILD_MODE").as_deref() == Ok("holder") {
        run_child_holder(&url).await;
        return;
    }

    // --- PARENT PROCESS TEST LOGIC ---
    let pool = sqlx::PgPool::connect(&url).await.expect("connect parent");
    ensure_disposable_marker(&pool)
        .await
        .expect("ensure marker");
    verify_disposable_database(&pool)
        .await
        .expect("verify disposable");

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

    // 2. Parent drops MaintenanceGuard
    println!("Parent Process: Dropping maintenance guard...");
    drop(guard);
    tokio::time::sleep(Duration::from_millis(50)).await;

    // 3. Independent OS Process B attempts writer permit again; must be admitted
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

    // 4. Test uncooperative OS process kill cleanup
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
    assert_eq!(excl_count, 0, "Exclusive lock must not be held");

    // Kill uncooperative Child Process C
    println!("Killing uncooperative Child Process C...");
    child_holder.kill().expect("kill child holder");
    let _ = child_holder.wait();

    // Give PostgreSQL engine a moment to tear down connection and release session locks
    tokio::time::sleep(Duration::from_millis(200)).await;

    // 5. Verify Parent can immediately acquire exclusive maintenance lock without blocking
    println!("Parent Process: Verifying exclusive maintenance acquisition succeeds cleanly...");
    let guard_after_kill = barrier
        .enter_maintenance(Duration::from_secs(5))
        .await
        .expect("acquire maintenance after child kill");
    println!("Parent Process: Exclusive maintenance guard acquired successfully after uncooperative process kill.");
    drop(guard_after_kill);

    println!("All Cross-Process Barrier Coordination tests PASSED.");
}
