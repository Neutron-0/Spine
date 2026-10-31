use spine_net::{handle_server_session, DEFAULT_SECRET_KEY};
use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_remote_configuration_and_tags() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Initialize repository
    let out = Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    // 2. Add remote
    let out = Command::new(spine_bin)
        .args([
            "remote",
            "add",
            "origin",
            "spine://127.0.0.1:4434/projects/backend",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Added remote 'origin'"));

    // 3. List remotes
    let out = Command::new(spine_bin)
        .args(["remote", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("origin\tspine://127.0.0.1:4434/projects/backend"));

    // 4. Get URL
    let out = Command::new(spine_bin)
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "spine://127.0.0.1:4434/projects/backend"
    );

    // 5. Set URL
    let out = Command::new(spine_bin)
        .args([
            "remote",
            "set-url",
            "origin",
            "spine://127.0.0.1:4435/new-projects/backend",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        "spine://127.0.0.1:4435/new-projects/backend"
    );

    // 6. Remove remote
    let out = Command::new(spine_bin)
        .args(["remote", "remove", "origin"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["remote", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(String::from_utf8_lossy(&out.stdout).contains("No remotes configured"));

    // 7. Make a commit for tag testing
    fs::write(root.join("hello.txt"), "hello tags\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "hello.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Tag commit"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    // 8. Create tag
    let out = Command::new(spine_bin)
        .args(["tag", "create", "v1.0.0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Created tag 'v1.0.0'"));

    // 9. List tags
    let out = Command::new(spine_bin)
        .args(["tag"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("v1.0.0"));

    // 10. Delete tag
    let out = Command::new(spine_bin)
        .args(["tag", "delete", "v1.0.0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["tag"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!String::from_utf8_lossy(&out.stdout).contains("v1.0.0"));
}

#[test]
fn test_loopback_clone_push_fetch_pull_and_cas_conflict() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");

    let server_root = tempdir().unwrap();
    let server_repo_dir = server_root.path().join("demo-repo");
    fs::create_dir_all(&server_repo_dir).unwrap();

    // 1. Initialize server repository and create initial commit
    let out = Command::new(spine_bin)
        .arg("init")
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    fs::write(server_repo_dir.join("README.md"), "# Spine Remote Demo\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "README.md"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Initial remote commit"])
        .current_dir(&server_repo_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // 2. Start server daemon listener in a background OS thread with its own runtime
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
    let remote_url = format!("spine://127.0.0.1:{}/demo-repo", port);

    // 3. Client 1: clone
    let client1_parent = tempdir().unwrap();
    let client1_dir = client1_parent.path().join("client1_repo");

    let out = Command::new(spine_bin)
        .args(["clone", &remote_url, client1_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(client1_dir.join("README.md").is_file());
    assert_eq!(
        fs::read_to_string(client1_dir.join("README.md")).unwrap(),
        "# Spine Remote Demo\n"
    );

    // 4. Client 1: make commit C2 and push
    fs::write(client1_dir.join("feature.txt"), "feature v1\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "feature.txt"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Feature from client 1"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "push stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 5. Client 2: clone and verify Feature commit C2 is received
    let client2_parent = tempdir().unwrap();
    let client2_dir = client2_parent.path().join("client2_repo");

    let out = Command::new(spine_bin)
        .args(["clone", &remote_url, client2_dir.to_str().unwrap()])
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(client2_dir.join("feature.txt").is_file());
    assert_eq!(
        fs::read_to_string(client2_dir.join("feature.txt")).unwrap(),
        "feature v1\n"
    );

    // 6. Client 2: make commit C3 and push
    fs::write(client2_dir.join("collaborator.txt"), "collab work\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "collaborator.txt"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Collaborator commit C3"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // 7. Client 1: pull and verify fast-forward update
    let out = Command::new(spine_bin)
        .args(["pull", "origin", "main"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "pull stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(client1_dir.join("collaborator.txt").is_file());
    assert_eq!(
        fs::read_to_string(client1_dir.join("collaborator.txt")).unwrap(),
        "collab work\n"
    );

    // 8. Test Non-Fast-Forward Rejection:
    // In Client 1, undo back one commit or create a divergent commit
    fs::write(client1_dir.join("divergent1.txt"), "divergent 1\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "divergent1.txt"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Divergent commit from client 1"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // Meanwhile, Client 2 makes a competing commit on main and pushes
    fs::write(client2_dir.join("competing2.txt"), "competing 2\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "competing2.txt"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Competing commit from client 2"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client2_dir)
        .output()
        .unwrap();
    assert!(out.status.success());

    // Now Client 1 tries to push its divergent commit without pulling -> MUST BE REJECTED!
    let out = Command::new(spine_bin)
        .args(["push", "origin", "main"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(
        !out.status.success(),
        "Divergent push should have been rejected!"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("non-fast-forward") || stderr.contains("rejected"),
        "stderr: {}",
        stderr
    );

    // 9. Force push succeeds when explicitly requested
    let out = Command::new(spine_bin)
        .args(["push", "origin", "main", "--force"])
        .current_dir(&client1_dir)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "Force push should succeed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[tokio::test]
async fn test_interrupted_pack_transfer_cleanup() {
    let temp = tempdir().unwrap();
    let spine_dir = temp.path().join(".spine");
    fs::create_dir_all(&spine_dir).unwrap();

    let store = spine_object_store::CompositeObjectStore::new(
        &spine_dir,
        spine_core::HashAlgorithm::Blake3,
        3,
    )
    .unwrap();

    // Create a mock stream that sends PackData followed by EOF without PackEnd
    let frame = spine_protocol::frame::Frame::new(
        spine_protocol::frame::FrameType::PackData,
        vec![1, 2, 3, 4, 5],
    )
    .unwrap();
    let frame_bytes = frame.serialize_to_vec();

    // Call receive_and_ingest_pack on truncated stream
    let err =
        spine_net::pack_transfer::receive_and_ingest_pack(&frame_bytes[..], &spine_dir, &store)
            .await
            .unwrap_err();

    // Verify it failed cleanly
    match err {
        spine_net::pack_transfer::TransferError::Aborted
        | spine_net::pack_transfer::TransferError::Protocol(_) => {}
        other => panic!("Expected Aborted or Protocol error, got: {:?}", other),
    }

    // Verify no temporary .incoming-* files remain in .spine/objects/pack/
    let pack_dir = spine_dir.join("objects").join("pack");
    if pack_dir.exists() {
        for entry in fs::read_dir(pack_dir).unwrap().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            assert!(
                !name.starts_with(".incoming"),
                "Leftover incoming file: {}",
                name
            );
        }
    }
}
