mod common;

use common::HarnessDisposableDb;
use spine_api::backup::{CommitDecisionRecord, RestoreJournal, RestorePhase};
use spine_api::coordinator::{ClusterNodeClient, CoordinatorClient};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use tempfile::tempdir;

struct ChildGuard(Vec<Child>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        for child in &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn compute_file_sha256_hex(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut file = fs::File::open(path).unwrap_or_else(|e| {
        panic!("Failed to open release binary '{}': {}", path.display(), e);
    });
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).expect("read release binary");
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>()
}

fn find_verified_release_binary(name: &str) -> (PathBuf, String) {
    let exe_name = if cfg!(windows) {
        format!("{}.exe", name)
    } else {
        name.to_string()
    };
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("repo root");
    let rc_dir = repo_root.join("target").join("release-candidate");
    let manifest_path = rc_dir.join("SHA256SUMS");
    let manifest_text = fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
        panic!(
            "Release checksum manifest '{}' not found or unreadable ({}); build and stage release binaries first",
            manifest_path.display(),
            e
        )
    });
    let expected_sha256 = manifest_text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            let file = parts.next()?;
            if file == exe_name {
                Some(hash.to_ascii_lowercase())
            } else {
                None
            }
        })
        .next()
        .unwrap_or_else(|| {
            panic!(
                "No SHA256SUMS entry for '{}' in '{}'",
                exe_name,
                manifest_path.display()
            )
        });

    let bin_path = rc_dir.join(&exe_name);
    if !bin_path.exists() {
        panic!(
            "Release binary '{}' not found (debug binaries are disallowed for release verification); build and stage release binaries first",
            bin_path.display()
        );
    }

    let actual_sha256 = compute_file_sha256_hex(&bin_path);
    assert_eq!(
        actual_sha256,
        expected_sha256,
        "Release binary '{}' SHA-256 mismatch: actual {} != manifest {} in '{}'",
        bin_path.display(),
        actual_sha256,
        expected_sha256,
        manifest_path.display()
    );
    println!(
        "VERIFIED RELEASE BINARY: {} sha256={}",
        bin_path.display(),
        actual_sha256
    );
    (bin_path, actual_sha256)
}

fn pick_free_addr() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().to_string()
}

async fn wait_for_node(addr: &str) {
    let client = ClusterNodeClient::new(addr.to_string());
    for _ in 0..100 {
        if client.query_node_details().await.is_ok() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("Timed out waiting for live cluster process at {}", addr);
}

#[tokio::test]
async fn test_combined_production_shaped_postgres_and_cluster_recovery() {
    if std::env::var("SPINE_TEST_DATABASE_URL")
        .map(|v| v.trim().is_empty())
        .unwrap_or(true)
    {
        eprintln!(
            "Skipping combined production-shaped restore test: SPINE_TEST_DATABASE_URL not set"
        );
        return;
    }

    // 1. Provision isolated disposable PostgreSQL database and set _restore_generation = "20"
    let disp_db = HarnessDisposableDb::provision()
        .await
        .expect("provision disposable PostgreSQL database");
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS _restore_generation (id INT PRIMARY KEY DEFAULT 1, generation_token TEXT NOT NULL)",
    )
    .execute(&disp_db.pool)
    .await
    .expect("create _restore_generation");
    sqlx::query(
        "INSERT INTO _restore_generation (id, generation_token) VALUES (1, '20') ON CONFLICT (id) DO UPDATE SET generation_token = '20'",
    )
    .execute(&disp_db.pool)
    .await
    .expect("seed _restore_generation");

    // 2. Setup local storage + SSH participants + leftover old dirs to verify finalization cleanup
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("api_storage");
    let staging = root.join(".staged_combined");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();
    let ssh_dir = root.join("ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::write(ssh_dir.join("host_key_ed25519"), b"combined-e2e-ssh-key").unwrap();
    RestoreJournal::write_storage_generation(&root, "20").unwrap();

    // 3. Start verified release spine-coordinator + 3 spine-storage-node processes
    let (coord_bin, _coord_sha256) = find_verified_release_binary("spine-coordinator");
    let (node_bin, _node_sha256) = find_verified_release_binary("spine-storage-node");

    let coord_store_path = tmp.path().join("coord").join("coordinator_state.json");
    fs::create_dir_all(coord_store_path.parent().unwrap()).unwrap();
    let coord_addr = pick_free_addr();

    let n1_dir = tmp.path().join("node-1");
    let n2_dir = tmp.path().join("node-2");
    let n3_dir = tmp.path().join("node-3");
    fs::create_dir_all(&n1_dir).unwrap();
    fs::create_dir_all(&n2_dir).unwrap();
    fs::create_dir_all(&n3_dir).unwrap();
    let n1_addr = pick_free_addr();
    let n2_addr = pick_free_addr();
    let n3_addr = pick_free_addr();

    let spawn_coord = |addr: &str| {
        Command::new(&coord_bin)
            .env("SPINE_CLUSTER_ID", "combined-e2e-cluster")
            .env("SPINE_LISTEN", addr)
            .env("SPINE_STORE_PATH", &coord_store_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spine-coordinator")
    };
    let spawn_node = |id: &str, addr: &str, dir: &Path| {
        Command::new(&node_bin)
            .env("SPINE_NODE_ID", id)
            .env("SPINE_LISTEN", addr)
            .env("SPINE_DATA_DIR", dir)
            .env("SPINE_COORDINATOR_ADDR", &coord_addr)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spine-storage-node")
    };

    let mut guards = ChildGuard(vec![
        spawn_coord(&coord_addr),
        spawn_node("node-1", &n1_addr, &n1_dir),
        spawn_node("node-2", &n2_addr, &n2_dir),
        spawn_node("node-3", &n3_addr, &n3_dir),
    ]);

    wait_for_node(&coord_addr).await;
    wait_for_node(&n1_addr).await;
    wait_for_node(&n2_addr).await;
    wait_for_node(&n3_addr).await;

    let coord_client = CoordinatorClient::new(coord_addr.clone());
    let n1_client = ClusterNodeClient::new(n1_addr.clone());
    let n2_client = ClusterNodeClient::new(n2_addr.clone());
    let n3_client = ClusterNodeClient::new(n3_addr.clone());

    // Capture initial snapshots and stage target generation 20 across all 4 cluster processes
    let (mut cm, cp) = coord_client.capture_snapshot("cap_c", 1).await.unwrap();
    let (mut m1, p1) = n1_client.capture_snapshot("cap_1", 1).await.unwrap();
    let (mut m2, p2) = n2_client.capture_snapshot("cap_2", 1).await.unwrap();
    let (mut m3, p3) = n3_client.capture_snapshot("cap_3", 1).await.unwrap();

    coord_client.enter_maintenance(20).await.unwrap();
    n1_client.enter_maintenance(20).await.unwrap();
    n2_client.enter_maintenance(20).await.unwrap();
    n3_client.enter_maintenance(20).await.unwrap();

    cm.snapshot_id = "snap_comb_coord".to_string();
    cm.fencing_generation = 20;
    m1.snapshot_id = "snap_comb_n1".to_string();
    m1.fencing_generation = 20;
    m2.snapshot_id = "snap_comb_n2".to_string();
    m2.fencing_generation = 20;
    m3.snapshot_id = "snap_comb_n3".to_string();
    m3.fencing_generation = 20;

    coord_client.stage_restore(cm, cp).await.unwrap();
    n1_client.stage_restore(m1, p1).await.unwrap();
    n2_client.stage_restore(m2, p2).await.unwrap();
    n3_client.stage_restore(m3, p3).await.unwrap();

    // Fault 1: Crash and restart coordinator + node-2 AFTER staging (before commit)
    let _ = guards.0[0].kill();
    let _ = guards.0[0].wait();
    let _ = guards.0[2].kill();
    let _ = guards.0[2].wait();
    guards.0[0] = spawn_coord(&coord_addr);
    guards.0[2] = spawn_node("node-2", &n2_addr, &n2_dir);
    wait_for_node(&coord_addr).await;
    wait_for_node(&n2_addr).await;

    // Fault 2: Commit coordinator + node-1, then crash and restart node-1 BEFORE finalize
    coord_client.enter_maintenance(20).await.unwrap();
    coord_client
        .commit_restore("snap_comb_coord", 20)
        .await
        .unwrap();
    n1_client.commit_restore("snap_comb_n1", 20).await.unwrap();
    let _ = guards.0[1].kill();
    let _ = guards.0[1].wait();
    guards.0[1] = spawn_node("node-1", &n1_addr, &n1_dir);
    wait_for_node(&n1_addr).await;

    // Fault 3: Kill node-3 right before first recovery run so recovery fails mid-cluster
    let _ = guards.0[3].kill();
    let _ = guards.0[3].wait();

    // Persist Committed journal + leftover old dirs to verify finalization cleanup
    let mut journal = RestoreJournal::new(
        &tmp.path().join("combined.spnebak"),
        &root,
        &staging,
        "bak_combined",
    );
    journal.target_generation = "20".to_string();
    journal
        .record_participant_snapshot("coordinator", "snap_comb_coord")
        .unwrap();
    journal
        .record_participant_snapshot("node-1", "snap_comb_n1")
        .unwrap();
    journal
        .record_participant_snapshot("node-2", "snap_comb_n2")
        .unwrap();
    journal
        .record_participant_snapshot("node-3", "snap_comb_n3")
        .unwrap();

    let old_gen_dir = root.join(format!(".old_generation_{}", journal.restore_id));
    let old_ssh_dir = root.join(format!(".old_ssh_{}", journal.restore_id));
    fs::create_dir_all(&old_gen_dir).unwrap();
    fs::create_dir_all(&old_ssh_dir).unwrap();

    let cdr = CommitDecisionRecord {
        restore_id: journal.restore_id.clone(),
        backup_id: "bak_combined".to_string(),
        old_generation: Some("1".to_string()),
        new_generation: "20".to_string(),
        required_participants: vec![
            "postgres_data".to_string(),
            "spine_storage".to_string(),
            "spine_ssh".to_string(),
            "coordinator".to_string(),
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_combined".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();
    journal.persist(RestorePhase::Committed).unwrap();

    let nodes = vec![
        ("node-1".to_string(), n1_addr.clone()),
        ("node-2".to_string(), n2_addr.clone()),
        ("node-3".to_string(), n3_addr.clone()),
    ];

    // Recovery Run 1 (using real PostgreSQL app_pool): must fail closed on downed node-3 while persisting prior progress
    let run1 = RestoreJournal::handle_startup_recovery_cluster(
        &root,
        Some(&disp_db.app_pool),
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(run1.is_err(), "Run 1 must fail closed while node-3 is down");
    let partial = RestoreJournal::load(&root)
        .unwrap()
        .expect("Journal must persist across mid-recovery failure");
    assert!(partial.participant_progress["postgres_data"].finalized);
    assert!(partial.participant_progress["spine_ssh"].finalized);
    assert!(partial.participant_progress["coordinator"].finalized);
    assert!(partial.participant_progress["node-1"].finalized);
    assert!(partial.participant_progress["node-2"].finalized);
    assert!(
        old_gen_dir.exists(),
        "Finalization should not remove old_gen_dir until all participants converge"
    );
    assert!(
        !old_ssh_dir.exists(),
        "Old SSH dir cleaned on spine_ssh finalization"
    );

    // Restart node-3 and execute Recovery Run 2: all 7 participants must converge and finalize
    guards.0[3] = spawn_node("node-3", &n3_addr, &n3_dir);
    wait_for_node(&n3_addr).await;

    let run2 = RestoreJournal::handle_startup_recovery_cluster(
        &root,
        Some(&disp_db.app_pool),
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(
        run2.is_ok(),
        "Run 2 must converge and finalize all 7 participants: {:?}",
        run2
    );
    assert!(
        RestoreJournal::load(&root).unwrap().is_none(),
        "Journal must be removed after global convergence"
    );
    assert!(!old_gen_dir.exists());
    assert!(!old_ssh_dir.exists());

    // Verify all 7 participants' final state
    let pg_gen: (String,) =
        sqlx::query_as("SELECT generation_token FROM _restore_generation WHERE id = 1")
            .fetch_one(&disp_db.app_pool)
            .await
            .unwrap();
    assert_eq!(pg_gen.0, "20");
    assert_eq!(
        RestoreJournal::read_storage_generation(&root).as_deref(),
        Some("20")
    );
    assert!(ssh_dir.join("host_key_ed25519").exists());

    for (label, client) in [
        ("coordinator", &ClusterNodeClient::new(coord_addr)),
        ("node-1", &n1_client),
        ("node-2", &n2_client),
        ("node-3", &n3_client),
    ] {
        let (maint, gen, _, id) = client.query_node_details().await.unwrap();
        assert!(!maint, "{} remained in maintenance after recovery", label);
        assert_eq!(gen, 20, "{} generation mismatch after recovery", label);
        assert_eq!(id.as_deref(), Some(label));
    }

    // Recovery Run 3 (idempotency on subsequent restart): must succeed cleanly as a no-op
    let run3 = RestoreJournal::handle_startup_recovery_cluster(
        &root,
        Some(&disp_db.app_pool),
        Some(&coord_client),
        &nodes,
    )
    .await
    .expect("Subsequent restart recovery must be idempotent");
    assert!(run3.is_none());

    disp_db.cleanup().await.expect("cleanup disposable db");
}
