mod common;

use common::HarnessDisposableDb;
use sha2::{Digest, Sha256};
use spine_api::backup::{run_backup_with_maintenance_cleanup, ConsistencyBarrier};
use spine_api::coordinator::{ClusterNodeClient, CoordinatorClient};
use spine_api::core::error::ForgeError;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

struct MockBarrierNode {
    in_maintenance: AtomicBool,
    fencing_generation: AtomicU64,
    drop_enter_ack_once: AtomicBool,
    fail_exit: AtomicBool,
}

impl MockBarrierNode {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            in_maintenance: AtomicBool::new(false),
            fencing_generation: AtomicU64::new(1),
            drop_enter_ack_once: AtomicBool::new(false),
            fail_exit: AtomicBool::new(false),
        })
    }
}

async fn spawn_mock_barrier_server(state: Arc<MockBarrierNode>) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_c = Arc::clone(&stop);

    tokio::spawn(async move {
        while !stop_c.load(Ordering::SeqCst) {
            if let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_millis(50), listener.accept()).await
            {
                let st = Arc::clone(&state);
                tokio::spawn(async move {
                    let mut lb = [0u8; 4];
                    if stream.read_exact(&mut lb).await.is_err() {
                        return;
                    }
                    let len = u32::from_be_bytes(lb) as usize;
                    let mut body = vec![0u8; len];
                    if stream.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let Ok(req) = ClusterRpcRequest::from_bytes(&body) else {
                        return;
                    };
                    let resp = match req {
                        ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation } => {
                            st.fencing_generation
                                .store(fencing_generation, Ordering::SeqCst);
                            st.in_maintenance.store(true, Ordering::SeqCst);
                            if st.drop_enter_ack_once.swap(false, Ordering::SeqCst) {
                                // Simulate lost acknowledgement after participant applied maintenance
                                drop(stream);
                                return;
                            }
                            ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                        }
                        ClusterRpcRequest::ExitMaintenanceBarrier { fencing_generation } => {
                            if st.fail_exit.load(Ordering::SeqCst) {
                                ClusterRpcResponse::Error(
                                    "Injected exit_maintenance failure".to_string(),
                                )
                            } else {
                                st.fencing_generation
                                    .store(fencing_generation, Ordering::SeqCst);
                                st.in_maintenance.store(false, Ordering::SeqCst);
                                ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                            }
                        }
                        _ => ClusterRpcResponse::Error("Unsupported".to_string()),
                    };
                    if let Ok(rb) = resp.to_bytes() {
                        let _ = stream.write_all(&(rb.len() as u32).to_be_bytes()).await;
                        let _ = stream.write_all(&rb).await;
                        let _ = stream.flush().await;
                    }
                });
            }
        }
    });

    (addr, stop)
}

type NoGuardFn = fn() -> std::future::Ready<Result<(), ForgeError>>;

#[tokio::test]
async fn test_backup_cleanup_lost_enter_maintenance_ack_still_exits() {
    let coord_state = MockBarrierNode::new();
    let node_state = MockBarrierNode::new();
    node_state.drop_enter_ack_once.store(true, Ordering::SeqCst);

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let no_guard: Option<NoGuardFn> = None;
    let err = run_backup_with_maintenance_cleanup(
        42,
        Some(&coord_addr),
        &nodes,
        no_guard,
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("node-1"));
    assert!(!coord_state.in_maintenance.load(Ordering::SeqCst));
    assert!(
        !node_state.in_maintenance.load(Ordering::SeqCst),
        "node-1 must be exited from maintenance even though its enter_maintenance ACK was lost"
    );

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_backup_cleanup_coordinator_exit_failure_fails_command() {
    let coord_state = MockBarrierNode::new();
    coord_state.fail_exit.store(true, Ordering::SeqCst);
    let node_state = MockBarrierNode::new();

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let no_guard: Option<NoGuardFn> = None;
    let err = run_backup_with_maintenance_cleanup(
        43,
        Some(&coord_addr),
        &nodes,
        no_guard,
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("coordinator"));
    assert!(msg.contains("Injected exit_maintenance failure"));
    assert!(!node_state.in_maintenance.load(Ordering::SeqCst));

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_backup_cleanup_storage_node_exit_failure_fails_command() {
    let coord_state = MockBarrierNode::new();
    let node_state = MockBarrierNode::new();
    node_state.fail_exit.store(true, Ordering::SeqCst);

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let no_guard: Option<NoGuardFn> = None;
    let err = run_backup_with_maintenance_cleanup(
        44,
        Some(&coord_addr),
        &nodes,
        no_guard,
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("node-1"));
    assert!(msg.contains("Injected exit_maintenance failure"));
    assert!(!coord_state.in_maintenance.load(Ordering::SeqCst));

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_backup_cleanup_database_guard_release_failure_fails_command() {
    let coord_state = MockBarrierNode::new();
    let node_state = MockBarrierNode::new();

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let failing_guard = Some(|| async {
        Err(ForgeError::Internal(
            "Injected DB guard release failure".to_string(),
        ))
    });
    let err = run_backup_with_maintenance_cleanup(
        45,
        Some(&coord_addr),
        &nodes,
        failing_guard,
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("postgres_maintenance_guard"));
    assert!(msg.contains("Injected DB guard release failure"));
    assert!(!coord_state.in_maintenance.load(Ordering::SeqCst));
    assert!(!node_state.in_maintenance.load(Ordering::SeqCst));

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_backup_cleanup_capture_failure_then_successful_cleanup() {
    let coord_state = MockBarrierNode::new();
    let node_state = MockBarrierNode::new();

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let ok_guard = Some(|| async { Ok(()) });
    let err = run_backup_with_maintenance_cleanup(
        46,
        Some(&coord_addr),
        &nodes,
        ok_guard,
        |_, _| async {
            Err(ForgeError::Internal(
                "Injected snapshot capture failure".to_string(),
            ))
        },
    )
    .await
    .unwrap_err();

    assert!(err
        .to_string()
        .contains("Injected snapshot capture failure"));
    assert!(!coord_state.in_maintenance.load(Ordering::SeqCst));
    assert!(!node_state.in_maintenance.load(Ordering::SeqCst));

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_backup_cleanup_capture_failure_then_cleanup_failure_retains_both() {
    let coord_state = MockBarrierNode::new();
    let node_state = MockBarrierNode::new();
    node_state.fail_exit.store(true, Ordering::SeqCst);

    let (coord_addr, coord_stop) = spawn_mock_barrier_server(Arc::clone(&coord_state)).await;
    let (node_addr, node_stop) = spawn_mock_barrier_server(Arc::clone(&node_state)).await;
    let nodes = vec![("node-1".to_string(), node_addr)];

    let ok_guard = Some(|| async { Ok(()) });
    let err = run_backup_with_maintenance_cleanup(
        47,
        Some(&coord_addr),
        &nodes,
        ok_guard,
        |_, _| async {
            Err(ForgeError::Internal(
                "Original snapshot capture failure".to_string(),
            ))
        },
    )
    .await
    .unwrap_err();

    let msg = err.to_string();
    assert!(msg.contains("Original snapshot capture failure"));
    assert!(msg.contains("Injected exit_maintenance failure"));
    assert!(msg.contains("node-1"));
    assert!(!coord_state.in_maintenance.load(Ordering::SeqCst));

    coord_stop.store(true, Ordering::SeqCst);
    node_stop.store(true, Ordering::SeqCst);
}

struct ChildGuard(Vec<Child>);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        for c in &mut self.0 {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

fn find_verified_release_binary(name: &str) -> PathBuf {
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
    let manifest_text = fs::read_to_string(&manifest_path).expect("read SHA256SUMS");
    let expected = manifest_text
        .lines()
        .filter_map(|line| {
            let mut parts = line.split_whitespace();
            let h = parts.next()?;
            let f = parts.next()?;
            if f == exe_name {
                Some(h.to_ascii_lowercase())
            } else {
                None
            }
        })
        .next()
        .expect("SHA256SUMS entry");
    let bin_path = rc_dir.join(&exe_name);
    assert!(
        bin_path.exists(),
        "Missing release binary {}",
        bin_path.display()
    );
    let mut file = fs::File::open(&bin_path).unwrap();
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let actual: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect();
    assert_eq!(actual, expected);
    println!(
        "VERIFIED RELEASE BINARY: {} sha256={}",
        bin_path.display(),
        actual
    );
    bin_path
}

fn pick_free_addr() -> String {
    let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    l.local_addr().unwrap().to_string()
}

async fn wait_for_node(addr: &str) {
    let client = ClusterNodeClient::new(addr.to_string());
    for _ in 0..100 {
        if client.query_node_details().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("Timed out waiting for {}", addr);
}

#[tokio::test]
#[ignore = "requires isolated SPINE_TEST_DATABASE_URL; run with -- --ignored in release gate"]
async fn test_backup_cleanup_live_processes_and_postgres_guard_release() {
    let db_url = std::env::var("SPINE_TEST_DATABASE_URL").unwrap_or_default();
    assert!(
        !db_url.trim().is_empty(),
        "SPINE_TEST_DATABASE_URL must be set to an isolated disposable PostgreSQL instance"
    );

    let disp_db = HarnessDisposableDb::provision().await.unwrap();
    let tmp = tempfile::tempdir().unwrap();
    let coord_bin = find_verified_release_binary("spine-coordinator");
    let node_bin = find_verified_release_binary("spine-storage-node");

    let coord_store = tmp.path().join("coord").join("coordinator_state.json");
    fs::create_dir_all(coord_store.parent().unwrap()).unwrap();
    let node_dir = tmp.path().join("node-1");
    fs::create_dir_all(&node_dir).unwrap();
    let coord_addr = pick_free_addr();
    let node_addr = pick_free_addr();

    let _guards = ChildGuard(vec![
        Command::new(&coord_bin)
            .env("SPINE_CLUSTER_ID", "cleanup-e2e")
            .env("SPINE_LISTEN", &coord_addr)
            .env("SPINE_STORE_PATH", &coord_store)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
        Command::new(&node_bin)
            .env("SPINE_NODE_ID", "node-1")
            .env("SPINE_LISTEN", &node_addr)
            .env("SPINE_DATA_DIR", &node_dir)
            .env("SPINE_COORDINATOR_ADDR", &coord_addr)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    ]);
    wait_for_node(&coord_addr).await;
    wait_for_node(&node_addr).await;

    let barrier = ConsistencyBarrier::with_pool(disp_db.pool.clone());
    let guard = barrier
        .enter_maintenance(Duration::from_secs(5))
        .await
        .unwrap();
    let nodes = vec![("node-1".to_string(), node_addr.clone())];

    // 1. Live processes + real Postgres guard + capture failure -> cleans up all 3 cleanly
    let err = run_backup_with_maintenance_cleanup(
        30,
        Some(&coord_addr),
        &nodes,
        Some(move || async move { guard.release().await }),
        |_, _| async { Err(ForgeError::Internal("Live capture aborted".to_string())) },
    )
    .await
    .unwrap_err();
    assert!(err.to_string().contains("Live capture aborted"));

    let (c_maint, c_gen, _, _) = CoordinatorClient::new(coord_addr.clone())
        .query_node_details()
        .await
        .unwrap();
    let (n_maint, n_gen, _, _) = ClusterNodeClient::new(node_addr.clone())
        .query_node_details()
        .await
        .unwrap();
    assert!(!c_maint && c_gen == 30);
    assert!(!n_maint && n_gen == 30);

    // 2. Live processes + real Postgres guard release failure (drop barrier table while guard held)
    let guard2 = barrier
        .enter_maintenance(Duration::from_secs(5))
        .await
        .unwrap();
    sqlx::query("DROP TABLE _cluster_maintenance_barrier")
        .execute(&disp_db.pool)
        .await
        .unwrap();

    let err2 = run_backup_with_maintenance_cleanup(
        31,
        Some(&coord_addr),
        &nodes,
        Some(move || async move { guard2.release().await }),
        |_, _| async { Ok(()) },
    )
    .await
    .unwrap_err();
    assert!(
        err2.to_string().contains("postgres_maintenance_guard"),
        "Expected DB guard release failure to surface, got: {}",
        err2
    );

    // Coordinator and storage node must still have exited maintenance cleanly
    let (c_maint2, _, _, _) = CoordinatorClient::new(coord_addr)
        .query_node_details()
        .await
        .unwrap();
    let (n_maint2, _, _, _) = ClusterNodeClient::new(node_addr)
        .query_node_details()
        .await
        .unwrap();
    assert!(!c_maint2);
    assert!(!n_maint2);

    disp_db.cleanup().await.unwrap();
}
