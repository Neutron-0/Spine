use spine_net::auth::{CapabilityTicket, Role, DEFAULT_SECRET_KEY};
use spine_net::handle_server_session;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_authenticated_e2e_remote_hosting_workflow() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let server_root = tempdir().unwrap();
    let server_repo_dir = server_root.path().join("alice").join("project");
    fs::create_dir_all(&server_repo_dir).unwrap();

    // 1. Initialize server repository and create initial commit
    let out = Command::new(spine_bin)
        .arg("init")
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    fs::write(
        server_repo_dir.join("README.md"),
        "# Spine Project\nHosted on Native Daemon\n",
    )
    .unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "README.md"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Initial hosted commit"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // 2. Start SPWP Daemon server
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

    // 3. Issue Capability Tickets
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    let validity = 3600 * 1_000_000_000i64;

    let write_ticket = CapabilityTicket::new(
        "usr_alice".to_string(),
        "alice".to_string(),
        "repo_proj".to_string(),
        "alice/project".to_string(),
        Role::Admin,
        validity,
        now,
    );
    let signed_write_ticket = write_ticket.encode_and_sign(DEFAULT_SECRET_KEY);

    let read_only_ticket = CapabilityTicket::new(
        "usr_bob".to_string(),
        "bob".to_string(),
        "repo_proj".to_string(),
        "alice/project".to_string(),
        Role::Reader,
        validity,
        now,
    );
    let signed_read_ticket = read_only_ticket.encode_and_sign(DEFAULT_SECRET_KEY);

    // 4. Client 1: Clone with Contributor Capability Ticket
    let client1_parent = tempdir().unwrap();
    let client1_dir = client1_parent.path().join("workspace_alice");

    let clone_url = format!("spine://127.0.0.1:{}/alice/project", port);

    let out = Command::new(spine_bin)
        .args(["clone", &clone_url, client1_dir.to_str().unwrap()])
        .env("SPINE_CAPABILITY_TICKET", &signed_write_ticket)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(client1_dir.join("README.md").is_file());

    // 5. Client 1: Add file and push with contributor ticket
    fs::write(
        client1_dir.join("contributor_work.rs"),
        "pub fn compute() -> i32 { 42 }\n",
    )
    .unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "contributor_work.rs"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Add compute function"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client1_dir)
        .env("SPINE_CAPABILITY_TICKET", &signed_write_ticket)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "Push failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 6. Reader-only client tries to push: must be rejected with authorization failure
    let client2_parent = tempdir().unwrap();
    let client2_dir = client2_parent.path().join("workspace_bob");

    let out = Command::new(spine_bin)
        .args(["clone", &clone_url, client2_dir.to_str().unwrap()])
        .env("SPINE_CAPABILITY_TICKET", &signed_read_ticket)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(client2_dir.join("contributor_work.rs").is_file());

    fs::write(client2_dir.join("hacked.txt"), "unauthorized change\n").unwrap();
    let _ = Command::new(spine_bin)
        .args(["add", "hacked.txt"])
        .current_dir(&client2_dir)
        .output();
    let _ = Command::new(spine_bin)
        .args(["commit", "-m", "Bob trying to push"])
        .current_dir(&client2_dir)
        .output();

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client2_dir)
        .env("SPINE_CAPABILITY_TICKET", &signed_read_ticket)
        .output()
        .unwrap();
    assert!(!out.status.success(), "Reader ticket push MUST be rejected");
}
