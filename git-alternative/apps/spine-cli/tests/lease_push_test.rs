//! Integration test suite for lease-based force push (`--force-with-lease`)
//! and concurrent update protection.

use spine_net::{handle_server_session, DEFAULT_SECRET_KEY};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_force_with_lease_workflow_and_rejection() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");

    let server_root = tempdir().unwrap();
    let server_repo_dir = server_root.path().join("lease-repo");
    fs::create_dir_all(&server_repo_dir).unwrap();

    // 1. Initialize server repository and create initial commit
    let out = Command::new(spine_bin)
        .arg("init")
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    fs::write(server_repo_dir.join("file.txt"), "Version 1\n").unwrap();
    let _ = Command::new(spine_bin)
        .args(["add", "file.txt"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    let _ = Command::new(spine_bin)
        .args(["commit", "-m", "Commit 1"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();

    // Get initial commit OID
    let out = Command::new(spine_bin)
        .args(["log"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&out.stdout);
    let commit1_oid = log_str
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_string();

    // 2. Start daemon in background OS thread with dedicated multi-thread runtime
    let (port_tx, port_rx) = std::sync::mpsc::channel();
    let daemon_root = server_root.path().to_path_buf();
    let secret = *DEFAULT_SECRET_KEY;

    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(async move {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let port = listener.local_addr().unwrap().port();
            port_tx.send(port).unwrap();

            while let Ok((socket, _)) = listener.accept().await {
                let root = daemon_root.clone();
                tokio::spawn(async move {
                    let _ = handle_server_session(socket, root, secret).await;
                });
            }
        });
    });

    let port = port_rx.recv().unwrap();
    let remote_url = format!("spine://127.0.0.1:{}/lease-repo", port);

    // 3. Clone to client A and client B
    let client_a_dir = tempdir().unwrap();
    let out = Command::new(spine_bin)
        .args(["clone", &remote_url, client_a_dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());

    let client_b_dir = tempdir().unwrap();
    let out = Command::new(spine_bin)
        .args(["clone", &remote_url, client_b_dir.path().to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());

    // 4. Client A creates Commit 2A and pushes
    fs::write(
        client_a_dir.path().join("file.txt"),
        "Version 2A from Client A\n",
    )
    .unwrap();
    let _ = Command::new(spine_bin)
        .args(["add", "file.txt"])
        .current_dir(client_a_dir.path())
        .output()
        .unwrap();
    let _ = Command::new(spine_bin)
        .args(["commit", "-m", "Commit 2A"])
        .current_dir(client_a_dir.path())
        .output()
        .unwrap();

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(client_a_dir.path())
        .output()
        .unwrap();
    assert!(out.status.success(), "Client A push should succeed");

    // 5. Client B creates Commit 2B (divergent history from Commit 1)
    fs::write(
        client_b_dir.path().join("file.txt"),
        "Version 2B from Client B\n",
    )
    .unwrap();
    let _ = Command::new(spine_bin)
        .args(["add", "file.txt"])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();
    let _ = Command::new(spine_bin)
        .args(["commit", "-m", "Commit 2B"])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();

    // 5a. Normal push by Client B should fail (non-fast-forward)
    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Client B push must be rejected as non-fast-forward"
    );

    // 5b. Push with stale lease (expecting commit1_oid, but remote is now at commit 2A!)
    // Must be rejected because remote tip was updated by Client A!
    let out = Command::new(spine_bin)
        .args([
            "push",
            "origin",
            "main",
            &format!("--force-with-lease={}", commit1_oid),
        ])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Push with stale lease must be rejected"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("stale reference") || stderr.contains("Lease mismatch"));

    // 6. Now client B pulls and integrates or updates lease to match current remote tip (Commit 2A)
    // Query remote log to find Commit 2A
    let out = Command::new(spine_bin)
        .args(["fetch", "origin"])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();
    assert!(out.status.success());

    let tracking_file = client_b_dir
        .path()
        .join(".spine")
        .join("refs")
        .join("remotes")
        .join("origin")
        .join("main");
    let current_remote_tip = fs::read_to_string(&tracking_file)
        .unwrap()
        .trim()
        .to_string();

    // 7. Force push with correct current lease succeeds!
    let out = Command::new(spine_bin)
        .args([
            "push",
            "origin",
            "main",
            &format!("--force-with-lease={}", current_remote_tip),
        ])
        .current_dir(client_b_dir.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "Push with matching lease must succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}
