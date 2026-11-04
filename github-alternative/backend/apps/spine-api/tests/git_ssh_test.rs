use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use spine_api::api::{build_router, AppState};
use spine_api::auth::pat::hash_pat;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::fs;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tower::ServiceExt;

async fn setup_test_server() -> (String, u16, Arc<dyn Database>, AppState, tempfile::TempDir) {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let mut state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let http_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let http_port = http_listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", http_port);
    state.public_origin = base_url.clone();

    // Start native pure-Rust SSH daemon on ephemeral port
    let (ssh_addr, _ssh_handle) = spine_api::start_ssh_server(
        state.clone(),
        "127.0.0.1:0".parse().unwrap(),
    )
    .await
    .expect("Failed to start SSH test server");

    let ssh_port = ssh_addr.port();

    let router = build_router(state.clone());
    tokio::spawn(async move {
        axum::serve(http_listener, router).await.unwrap();
    });

    (base_url, ssh_port, db, state, tmp)
}

fn generate_client_ssh_key(key_path: &Path) -> (String, String) {
    let key_str = key_path.to_str().unwrap();
    let status = Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-f", key_str])
        .status()
        .expect("Failed to execute ssh-keygen");
    assert!(status.success(), "ssh-keygen failed");

    let pub_path = format!("{}.pub", key_path.display());
    let pub_key = fs::read_to_string(&pub_path).expect("Failed to read public key");

    let parsed = russh::keys::ssh_key::PublicKey::from_openssh(&pub_key)
        .expect("Failed to parse public key");
    let fingerprint = parsed
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();

    (pub_key, fingerprint)
}

fn run_git_ssh_cmd(cwd: &Path, key_path: &Path, args: &[&str]) -> (bool, String, String) {
    let key_str = key_path.display().to_string().replace('\\', "/");
    let ssh_command = format!(
        "ssh -i \"{}\" -o UserKnownHostsFile=/dev/null -o StrictHostKeyChecking=no -o LogLevel=ERROR",
        key_str
    );
    let ssh_cmd_config = format!("core.sshCommand={}", ssh_command);

    let mut cmd_args = vec![
        "-c", "core.autocrlf=false",
        "-c", &ssh_cmd_config,
    ];
    cmd_args.extend_from_slice(args);

    let output = Command::new("git")
        .args(&cmd_args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", &ssh_command)
        .env("GIT_AUTHOR_NAME", "Spine SSH Tester")
        .env("GIT_AUTHOR_EMAIL", "tester@spine.build")
        .env("GIT_COMMITTER_NAME", "Spine SSH Tester")
        .env("GIT_COMMITTER_EMAIL", "tester@spine.build")
        .output()
        .expect("Failed to execute git over SSH");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

fn run_git_ssh_cmd_strict(
    cwd: &Path,
    key_path: &Path,
    known_hosts_path: &Path,
    args: &[&str],
) -> (bool, String, String) {
    let key_str = key_path.display().to_string().replace('\\', "/");
    let known_hosts_str = known_hosts_path.display().to_string().replace('\\', "/");
    let ssh_command = format!(
        "ssh -i \"{}\" -o UserKnownHostsFile=\"{}\" -o StrictHostKeyChecking=yes -o LogLevel=ERROR",
        key_str, known_hosts_str
    );
    let ssh_cmd_config = format!("core.sshCommand={}", ssh_command);

    let mut cmd_args = vec!["-c", "core.autocrlf=false", "-c", &ssh_cmd_config];
    cmd_args.extend_from_slice(args);

    let output = Command::new("git")
        .args(&cmd_args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_SSH_COMMAND", &ssh_command)
        .env("GIT_AUTHOR_NAME", "Spine SSH Strict Tester")
        .env("GIT_AUTHOR_EMAIL", "strict_tester@spine.build")
        .env("GIT_COMMITTER_NAME", "Spine SSH Strict Tester")
        .env("GIT_COMMITTER_EMAIL", "strict_tester@spine.build")
        .output()
        .expect("Failed to execute git over SSH");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_ssh_key_auth_and_push_clone_roundtrip() {
    let (_http_url, ssh_port, db, state, tmp) = setup_test_server().await;

    // 1. Create User
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice SSH".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    // 2. Generate client SSH key and register in Forge
    let key_path = tmp.path().join("id_ed25519");
    let (pub_key_str, fingerprint) = generate_client_ssh_key(&key_path);

    let ssh_key = UserSshKey {
        id: "key_alice_1".to_string(),
        user_id: user.id.clone(),
        title: "Alice Laptop".to_string(),
        fingerprint,
        public_key: pub_key_str,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_user_ssh_key(ssh_key).await.unwrap();

    // 3. Create Repository
    let _repo = state
        .repositories
        .create_repository(&user, "demo-ssh", Some("SSH roundtrip test".into()), false, None)
        .await
        .unwrap();

    // 4. Initialize local client repository and push over SSH
    let client_dir = tmp.path().join("client_push");
    fs::create_dir_all(&client_dir).unwrap();

    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["init", "-b", "main"]);
    assert!(ok, "git init failed: {}", err);

    fs::write(client_dir.join("README.md"), "# Spine Native Git-over-SSH\nWorks seamlessly!").unwrap();
    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["add", "."]);
    assert!(ok, "git add failed: {}", err);

    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["commit", "-m", "Initial SSH commit"]);
    assert!(ok, "git commit failed: {}", err);

    let ssh_remote = format!("ssh://git@127.0.0.1:{}/alice/demo-ssh.git", ssh_port);
    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["remote", "add", "origin", &ssh_remote]);
    assert!(ok, "git remote add failed: {}", err);

    let (ok, stdout, stderr) = run_git_ssh_cmd(&client_dir, &key_path, &["push", "-u", "origin", "main"]);
    assert!(ok, "git push over SSH failed: {}\n{}", stdout, stderr);

    // 5. Clone repository to another directory over SSH
    let clone_dir = tmp.path().join("client_clone");
    let (ok, stdout, stderr) = run_git_ssh_cmd(
        tmp.path(),
        &key_path,
        &["clone", &ssh_remote, clone_dir.to_str().unwrap()],
    );
    assert!(ok, "git clone over SSH failed: {}\n{}", stdout, stderr);

    let cloned_content = fs::read_to_string(clone_dir.join("README.md")).unwrap();
    assert_eq!(cloned_content, "# Spine Native Git-over-SSH\nWorks seamlessly!");

    // Verify commit log
    let (ok, log_out, _) = run_git_ssh_cmd(&clone_dir, &key_path, &["log", "-1", "--pretty=%s"]);
    assert!(ok);
    assert_eq!(log_out.trim(), "Initial SSH commit");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_ssh_deploy_keys_read_and_write_enforcement() {
    let (_http_url, ssh_port, db, state, tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let owner = User {
        id: "usr_carol".to_string(),
        username: "carol".to_string(),
        normalized_username: "carol".to_string(),
        email: "carol@spine.build".to_string(),
        normalized_email: "carol@spine.build".to_string(),
        display_name: "Carol Repo Owner".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(owner.clone()).await.unwrap();

    let repo = state
        .repositories
        .create_repository(&owner, "deploy-test", Some("Deploy keys test".into()), true, None)
        .await
        .unwrap();

    // Setup initial repository content using owner's SSH key
    let owner_key_path = tmp.path().join("id_owner");
    let (owner_pub, owner_fp) = generate_client_ssh_key(&owner_key_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_owner".to_string(),
        user_id: owner.id.clone(),
        title: "Owner Key".to_string(),
        fingerprint: owner_fp,
        public_key: owner_pub,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    let init_dir = tmp.path().join("init_repo");
    fs::create_dir_all(&init_dir).unwrap();
    run_git_ssh_cmd(&init_dir, &owner_key_path, &["init", "-b", "main"]);
    fs::write(init_dir.join("app.txt"), "hello deploy keys").unwrap();
    run_git_ssh_cmd(&init_dir, &owner_key_path, &["add", "."]);
    run_git_ssh_cmd(&init_dir, &owner_key_path, &["commit", "-m", "Initial commit"]);
    let remote_url = format!("ssh://git@127.0.0.1:{}/carol/deploy-test.git", ssh_port);
    run_git_ssh_cmd(&init_dir, &owner_key_path, &["remote", "add", "origin", &remote_url]);
    let (ok, _, err) = run_git_ssh_cmd(&init_dir, &owner_key_path, &["push", "-u", "origin", "main"]);
    assert!(ok, "Initial push failed: {}", err);

    // 1. Create Read-Only Deploy Key
    let ro_key_path = tmp.path().join("id_deploy_ro");
    let (ro_pub, ro_fp) = generate_client_ssh_key(&ro_key_path);
    db.create_deploy_key(DeployKey {
        id: "dp_ro".to_string(),
        repository_id: repo.id.clone(),
        title: "Read Only CI".to_string(),
        fingerprint: ro_fp,
        public_key: ro_pub,
        key_type: "ssh-ed25519".to_string(),
        write_enabled: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    // 2. Read-only deploy key can clone
    let ro_clone_dir = tmp.path().join("clone_ro");
    let (ok, _, err) = run_git_ssh_cmd(tmp.path(), &ro_key_path, &["clone", &remote_url, ro_clone_dir.to_str().unwrap()]);
    assert!(ok, "Deploy key clone failed: {}", err);
    assert_eq!(fs::read_to_string(ro_clone_dir.join("app.txt")).unwrap(), "hello deploy keys");

    // 3. Read-only deploy key CANNOT push
    fs::write(ro_clone_dir.join("hack.txt"), "forbidden write").unwrap();
    run_git_ssh_cmd(&ro_clone_dir, &ro_key_path, &["add", "."]);
    run_git_ssh_cmd(&ro_clone_dir, &ro_key_path, &["commit", "-m", "Illegal push"]);
    let (ok, _, err) = run_git_ssh_cmd(&ro_clone_dir, &ro_key_path, &["push", "origin", "main"]);
    assert!(!ok, "Read-only deploy key must NOT be permitted to push");
    assert!(err.contains("Write access denied") || err.contains("denied") || err.contains("FATAL") || err.contains("access rights"), "Expected write denial error: {}", err);

    // 4. Create Write-Enabled Deploy Key
    let rw_key_path = tmp.path().join("id_deploy_rw");
    let (rw_pub, rw_fp) = generate_client_ssh_key(&rw_key_path);
    db.create_deploy_key(DeployKey {
        id: "dp_rw".to_string(),
        repository_id: repo.id.clone(),
        title: "Read Write Bot".to_string(),
        fingerprint: rw_fp,
        public_key: rw_pub,
        key_type: "ssh-ed25519".to_string(),
        write_enabled: true,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    // 5. Write-enabled deploy key CAN push
    let (ok, stdout, stderr) = run_git_ssh_cmd(&ro_clone_dir, &rw_key_path, &["push", "origin", "main"]);
    assert!(ok, "Write-enabled deploy key push failed: {}\n{}", stdout, stderr);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_ssh_security_controls_and_secret_scanning() {
    let (_http_url, ssh_port, db, state, tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_sec".to_string(),
        username: "secuser".to_string(),
        normalized_username: "secuser".to_string(),
        email: "secuser@spine.build".to_string(),
        normalized_email: "secuser@spine.build".to_string(),
        display_name: "Security Tester".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let key_path = tmp.path().join("id_sec");
    let (pub_key_str, fingerprint) = generate_client_ssh_key(&key_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_sec".to_string(),
        user_id: user.id.clone(),
        title: "Sec Key".to_string(),
        fingerprint,
        public_key: pub_key_str,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    let _repo = state
        .repositories
        .create_repository(&user, "sec-test", Some("Security test repo".into()), false, None)
        .await
        .unwrap();

    // 1. Interactive shell attempt rejected
    let key_str = key_path.display().to_string().replace('\\', "/");
    let shell_out = Command::new("ssh")
        .args([
            "-i", &key_str,
            "-p", &ssh_port.to_string(),
            "-o", "UserKnownHostsFile=/dev/null",
            "-o", "StrictHostKeyChecking=no",
            "git@127.0.0.1",
        ])
        .output()
        .expect("Failed to execute ssh for shell test");
    assert!(!shell_out.status.success(), "Interactive SSH shell must be rejected");
    let stderr_shell = String::from_utf8_lossy(&shell_out.stderr);
    assert!(
        stderr_shell.contains("Interactive shell sessions are not permitted")
            || stderr_shell.contains("Permission denied")
            || stderr_shell.contains("shell request failed")
            || stderr_shell.contains("PTY allocation request failed"),
        "Unexpected error: {}",
        stderr_shell
    );

    // 2. Push with leaked AWS secret rejected by server-side secret scanning
    let client_dir = tmp.path().join("client_leak");
    fs::create_dir_all(&client_dir).unwrap();
    run_git_ssh_cmd(&client_dir, &key_path, &["init", "-b", "main"]);

    let leaked_content = "AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE\n";
    fs::write(client_dir.join("credentials.env"), leaked_content).unwrap();
    run_git_ssh_cmd(&client_dir, &key_path, &["add", "."]);
    run_git_ssh_cmd(&client_dir, &key_path, &["commit", "-m", "Commit with AWS key"]);

    let remote_url = format!("ssh://git@127.0.0.1:{}/secuser/sec-test.git", ssh_port);
    run_git_ssh_cmd(&client_dir, &key_path, &["remote", "add", "origin", &remote_url]);

    let (ok, stdout, stderr) = run_git_ssh_cmd(&client_dir, &key_path, &["push", "origin", "main"]);
    assert!(!ok, "Push with leaked secret must be rejected: {}\n{}", stdout, stderr);
    assert!(stderr.contains("secret") || stderr.contains("pre-receive hook declined") || stdout.contains("secret"),
        "Expected secret scanning rejection output: {}\n{}", stdout, stderr);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_lfs_ssh_authenticate_and_transfer() {
    let (_http_url, ssh_port, db, state, tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_lfs".to_string(),
        username: "lfsuser".to_string(),
        normalized_username: "lfsuser".to_string(),
        email: "lfs@spine.build".to_string(),
        normalized_email: "lfs@spine.build".to_string(),
        display_name: "LFS SSH Tester".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let key_path = tmp.path().join("id_lfs");
    let (pub_key_str, fingerprint) = generate_client_ssh_key(&key_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_lfs".to_string(),
        user_id: user.id.clone(),
        title: "LFS Key".to_string(),
        fingerprint,
        public_key: pub_key_str,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    let _repo = state
        .repositories
        .create_repository(&user, "lfs-repo", Some("LFS SSH Repo".into()), false, None)
        .await
        .unwrap();

    let client_dir = tmp.path().join("client_lfs");
    fs::create_dir_all(&client_dir).unwrap();
    run_git_ssh_cmd(&client_dir, &key_path, &["init", "-b", "main"]);

    // Initialize git lfs
    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["lfs", "install"]);
    assert!(ok, "git lfs install failed: {}", err);

    let (ok, _, err) = run_git_ssh_cmd(&client_dir, &key_path, &["lfs", "track", "*.dat"]);
    assert!(ok, "git lfs track failed: {}", err);

    let large_bytes = vec![0xABu8; 64 * 1024]; // 64 KB binary asset
    fs::write(client_dir.join("large_asset.dat"), &large_bytes).unwrap();

    run_git_ssh_cmd(&client_dir, &key_path, &["add", ".gitattributes", "large_asset.dat"]);
    run_git_ssh_cmd(&client_dir, &key_path, &["commit", "-m", "Add large asset with LFS"]);

    let remote_url = format!("ssh://git@127.0.0.1:{}/lfsuser/lfs-repo.git", ssh_port);
    run_git_ssh_cmd(&client_dir, &key_path, &["remote", "add", "origin", &remote_url]);

    // Push over SSH remote. Git LFS will automatically call git-lfs-authenticate over SSH!
    let (ok, stdout, stderr) = run_git_ssh_cmd(&client_dir, &key_path, &["push", "-u", "origin", "main"]);
    assert!(ok, "git push with LFS over SSH failed: {}\n{}", stdout, stderr);

    // Clone into fresh directory over SSH and verify LFS pull
    let clone_dir = tmp.path().join("clone_lfs");
    let (ok, stdout, stderr) = run_git_ssh_cmd(
        tmp.path(),
        &key_path,
        &["clone", &remote_url, clone_dir.to_str().unwrap()],
    );
    assert!(ok, "git clone with LFS over SSH failed: {}\n{}", stdout, stderr);

    let (ok, stdout, stderr) = run_git_ssh_cmd(&clone_dir, &key_path, &["lfs", "pull"]);
    assert!(ok, "git lfs pull failed: {}\n{}", stdout, stderr);

    let restored_bytes = fs::read(clone_dir.join("large_asset.dat")).unwrap();
    assert_eq!(restored_bytes.len(), 64 * 1024);
    assert_eq!(restored_bytes, large_bytes);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_lfs_file_locking_api_and_push_conflict() {
    let (_base_url, ssh_port, db, state, tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    // User Alice
    let alice = User {
        id: "usr_alice_lock".to_string(),
        username: "alice_lock".to_string(),
        normalized_username: "alice_lock".to_string(),
        email: "alice_lock@spine.build".to_string(),
        normalized_email: "alice_lock@spine.build".to_string(),
        display_name: "Alice Locks".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(alice.clone()).await.unwrap();

    let pat_alice = "sp_pat_alice_12345678901234567890123456789012";
    db.create_pat(PersonalAccessToken {
        id: "pat_alice".to_string(),
        user_id: alice.id.clone(),
        name: "Alice PAT".to_string(),
        token_hash: hash_pat(pat_alice),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: now + 3600 * 1_000_000_000,
        last_used_at_nanos: None,
        created_at_nanos: now,
        revoked: false,
    }).await.unwrap();

    let key_alice_path = tmp.path().join("id_alice");
    let (alice_pub, alice_fp) = generate_client_ssh_key(&key_alice_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_alice".to_string(),
        user_id: alice.id.clone(),
        title: "Alice Key".to_string(),
        fingerprint: alice_fp,
        public_key: alice_pub,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    // User Bob
    let bob = User {
        id: "usr_bob_lock".to_string(),
        username: "bob_lock".to_string(),
        normalized_username: "bob_lock".to_string(),
        email: "bob_lock@spine.build".to_string(),
        normalized_email: "bob_lock@spine.build".to_string(),
        display_name: "Bob Locks".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(bob.clone()).await.unwrap();

    let pat_bob = "sp_pat_bob_12345678901234567890123456789012";
    db.create_pat(PersonalAccessToken {
        id: "pat_bob".to_string(),
        user_id: bob.id.clone(),
        name: "Bob PAT".to_string(),
        token_hash: hash_pat(pat_bob),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: now + 3600 * 1_000_000_000,
        last_used_at_nanos: None,
        created_at_nanos: now,
        revoked: false,
    }).await.unwrap();

    let key_bob_path = tmp.path().join("id_bob");
    let (bob_pub, bob_fp) = generate_client_ssh_key(&key_bob_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_bob".to_string(),
        user_id: bob.id.clone(),
        title: "Bob Key".to_string(),
        fingerprint: bob_fp,
        public_key: bob_pub,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    // Repository owned by Alice
    let repo = state
        .repositories
        .create_repository(&alice, "locking-demo", Some("LFS locking test".into()), false, None)
        .await
        .unwrap();

    // Add Bob as a collaborator with write access to Alice's repository
    db.create_permission(RepositoryPermission {
        id: "perm_bob".to_string(),
        repo_id: repo.id.clone(),
        user_id: bob.id.clone(),
        role: PermissionRole::Collaborator,
        created_at_nanos: now,
    }).await.unwrap();

    let router = build_router(state.clone());
    let locks_uri = "/alice_lock/locking-demo.git/info/lfs/locks";

    // 1. Alice creates initial commit with binary file
    let alice_dir = tmp.path().join("alice_repo");
    fs::create_dir_all(&alice_dir).unwrap();
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["init", "-b", "main"]);
    fs::create_dir_all(alice_dir.join("models")).unwrap();
    fs::write(alice_dir.join("models/weights.bin"), b"v1 weights").unwrap();
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["add", "."]);
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["commit", "-m", "Add initial weights"]);
    let remote_url = format!("ssh://git@127.0.0.1:{}/alice_lock/locking-demo.git", ssh_port);
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["remote", "add", "origin", &remote_url]);
    let (ok, _, err) = run_git_ssh_cmd(&alice_dir, &key_alice_path, &["push", "-u", "origin", "main"]);
    assert!(ok, "Initial push failed: {}", err);

    // 2. Alice acquires lock on models/weights.bin via Git LFS Lock API
    let req = Request::builder()
        .method("POST")
        .uri(locks_uri)
        .header("Authorization", format!("Bearer {}", pat_alice))
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({ "path": "models/weights.bin" })).unwrap()))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let body_bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let lock_body: LfsCreateLockResponse = serde_json::from_slice(&body_bytes).unwrap();
    let lock_id = lock_body.lock.id;
    assert_eq!(lock_body.lock.path, "models/weights.bin");

    // 3. Bob tries to acquire lock on the same file -> 409 Conflict
    let req = Request::builder()
        .method("POST")
        .uri(locks_uri)
        .header("Authorization", format!("Bearer {}", pat_bob))
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({ "path": "models/weights.bin" })).unwrap()))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CONFLICT);

    // 4. List locks -> shows lock owned by Alice
    let req = Request::builder()
        .method("GET")
        .uri(locks_uri)
        .header("Authorization", format!("Bearer {}", pat_bob))
        .body(Body::empty())
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let body_bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let list_body: LfsListLocksResponse = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(list_body.locks.len(), 1);
    assert_eq!(list_body.locks[0].owner.name, "alice_lock");

    // 5. Bob clones and attempts to push modification to models/weights.bin
    let bob_dir = tmp.path().join("bob_repo");
    let (ok, _, err) = run_git_ssh_cmd(tmp.path(), &key_bob_path, &["clone", &remote_url, bob_dir.to_str().unwrap()]);
    assert!(ok, "Bob clone failed: {}", err);

    fs::write(bob_dir.join("models/weights.bin"), b"v2 bob conflicting weights").unwrap();
    run_git_ssh_cmd(&bob_dir, &key_bob_path, &["add", "."]);
    run_git_ssh_cmd(&bob_dir, &key_bob_path, &["commit", "-m", "Bob conflicting edit"]);

    let (ok, stdout, stderr) = run_git_ssh_cmd(&bob_dir, &key_bob_path, &["push", "origin", "main"]);
    assert!(!ok, "Bob push modifying locked file must be rejected: {}\n{}", stdout, stderr);
    assert!(stderr.contains("locked by") || stderr.contains("pre-receive hook declined") || stdout.contains("locked by"),
        "Expected lock conflict error: {}\n{}", stdout, stderr);

    // 6. Alice pushes modification to models/weights.bin -> succeeds because Alice holds the lock!
    fs::write(alice_dir.join("models/weights.bin"), b"v2 alice authorized weights").unwrap();
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["add", "."]);
    run_git_ssh_cmd(&alice_dir, &key_alice_path, &["commit", "-m", "Alice authorized update"]);
    let (ok, stdout, stderr) = run_git_ssh_cmd(&alice_dir, &key_alice_path, &["push", "origin", "main"]);
    assert!(ok, "Alice authorized push failed: {}\n{}", stdout, stderr);

    // 7. Bob attempts normal unlock on Alice's lock -> 403 Forbidden
    let unlock_uri = format!("/alice_lock/locking-demo.git/info/lfs/locks/{}/unlock", lock_id);
    let req = Request::builder()
        .method("POST")
        .uri(&unlock_uri)
        .header("Authorization", format!("Bearer {}", pat_bob))
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({ "force": false })).unwrap()))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    // 8. Alice unlocks the file -> 200 OK
    let req = Request::builder()
        .method("POST")
        .uri(&unlock_uri)
        .header("Authorization", format!("Bearer {}", pat_alice))
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({ "force": false })).unwrap()))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 9. Bob can now acquire lock
    let req = Request::builder()
        .method("POST")
        .uri(locks_uri)
        .header("Authorization", format!("Bearer {}", pat_bob))
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({ "path": "models/weights.bin" })).unwrap()))
        .unwrap();
    let res = router.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_ssh_strict_host_key_and_advanced_ops() {
    let (_http_url, ssh_port, db, state, tmp) = setup_test_server().await;
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 1. Create User
    let user = User {
        id: "usr_carol".to_string(),
        username: "carol".to_string(),
        normalized_username: "carol".to_string(),
        email: "carol@spine.build".to_string(),
        normalized_email: "carol@spine.build".to_string(),
        display_name: "Carol Strict".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let key_path = tmp.path().join("id_carol");
    let (carol_pub, carol_fp) = generate_client_ssh_key(&key_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_carol".to_string(),
        user_id: user.id.clone(),
        title: "Carol Key".to_string(),
        fingerprint: carol_fp,
        public_key: carol_pub,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    // 2. Read server's public host key and populate strict known_hosts
    let host_pub_path = tmp.path().join("storage/ssh/host_key_ed25519.pub");
    assert!(host_pub_path.exists(), "Host public key must exist on disk");
    let host_pub_key = fs::read_to_string(&host_pub_path).unwrap();

    let known_hosts_path = tmp.path().join("known_hosts_strict");
    let known_hosts_entry = format!("[127.0.0.1]:{} {}\n", ssh_port, host_pub_key.trim());
    fs::write(&known_hosts_path, &known_hosts_entry).unwrap();

    // 3. Create Public Repository and Initial Push over Strict SSH
    let _repo = state
        .repositories
        .create_repository(&user, "strict-repo", Some("Strict host key test".into()), false, None)
        .await
        .unwrap();

    let client_dir = tmp.path().join("carol_repo");
    fs::create_dir_all(&client_dir).unwrap();
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["init", "-b", "main"]);
    fs::write(client_dir.join("main.rs"), b"fn main() {}\n").unwrap();
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["add", "."]);
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["commit", "-m", "Initial commit"]);

    let remote_url = format!("ssh://git@127.0.0.1:{}/carol/strict-repo.git", ssh_port);
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["remote", "add", "origin", &remote_url]);

    // Push main branch with StrictHostKeyChecking=yes
    let (ok, out, err) = run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["push", "-u", "origin", "main"]);
    assert!(ok, "Strict SSH push failed:\nSTDOUT: {}\nSTDERR: {}", out, err);

    // 4. Branch Push
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["checkout", "-b", "feature-x"]);
    fs::write(client_dir.join("feature.txt"), b"feature content\n").unwrap();
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["add", "."]);
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["commit", "-m", "Feature commit"]);
    let (ok, out, err) = run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["push", "origin", "feature-x"]);
    assert!(ok, "Branch push failed:\nSTDOUT: {}\nSTDERR: {}", out, err);

    // 5. Tag Push
    run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["tag", "v1.0.0"]);
    let (ok, out, err) = run_git_ssh_cmd_strict(&client_dir, &key_path, &known_hosts_path, &["push", "origin", "v1.0.0"]);
    assert!(ok, "Tag push failed:\nSTDOUT: {}\nSTDERR: {}", out, err);

    // 6. Strict Clone into New Directory
    let clone_dir = tmp.path().join("carol_clone");
    let (ok, out, err) = run_git_ssh_cmd_strict(tmp.path(), &key_path, &known_hosts_path, &["clone", &remote_url, clone_dir.to_str().unwrap()]);
    assert!(ok, "Strict SSH clone failed:\nSTDOUT: {}\nSTDERR: {}", out, err);
    assert!(clone_dir.join("main.rs").exists());

    // 7. Fetch and Pull
    let (ok, out, err) = run_git_ssh_cmd_strict(&clone_dir, &key_path, &known_hosts_path, &["fetch", "origin"]);
    assert!(ok, "Strict SSH fetch failed:\nSTDOUT: {}\nSTDERR: {}", out, err);

    let (ok, out, err) = run_git_ssh_cmd_strict(&clone_dir, &key_path, &known_hosts_path, &["pull", "origin", "main"]);
    assert!(ok, "Strict SSH pull failed:\nSTDOUT: {}\nSTDERR: {}", out, err);

    // 8. Strict Host Key Verification Failure Test (Tampered Host Key)
    let bad_known_hosts = tmp.path().join("known_hosts_bad");
    fs::write(&bad_known_hosts, format!("[127.0.0.1]:{} ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIFakeHostKeyThatDoesNotMatchServerFingerprintXX\n", ssh_port)).unwrap();
    let (ok, out, err) = run_git_ssh_cmd_strict(&clone_dir, &key_path, &bad_known_hosts, &["fetch", "origin"]);
    assert!(!ok, "Connection with mismatched host key MUST fail strict verification");
    assert!(err.contains("Host key verification failed") || err.contains("REMOTE HOST IDENTIFICATION HAS CHANGED") || err.contains("fatal: Could not read"),
        "Expected host key verification error:\nSTDOUT: {}\nSTDERR: {}", out, err);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_ssh_persistence_and_restart() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    // 1. First Server Boot: Generates host key
    let (addr1, _handle1) = spine_api::start_ssh_server(state.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .expect("Server 1 boot");
    let port1 = addr1.port();

    let host_pub_path = storage_root.join("ssh/host_key_ed25519.pub");
    assert!(host_pub_path.exists());
    let pub1 = fs::read_to_string(&host_pub_path).unwrap();

    // Register User & Key
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_restart".to_string(),
        username: "restart_user".to_string(),
        normalized_username: "restart_user".to_string(),
        email: "restart@spine.build".to_string(),
        normalized_email: "restart@spine.build".to_string(),
        display_name: "Restart User".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let key_path = tmp.path().join("id_restart");
    let (pub_key_str, fingerprint) = generate_client_ssh_key(&key_path);
    db.create_user_ssh_key(UserSshKey {
        id: "key_restart".to_string(),
        user_id: user.id.clone(),
        title: "Restart Key".to_string(),
        fingerprint,
        public_key: pub_key_str,
        key_type: "ssh-ed25519".to_string(),
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    }).await.unwrap();

    let _repo = state
        .repositories
        .create_repository(&user, "restart-repo", Some("Restart persistence test".into()), false, None)
        .await
        .unwrap();

    // Push initial commit to server 1
    let init_dir = tmp.path().join("init_client");
    fs::create_dir_all(&init_dir).unwrap();
    let known_hosts_1 = tmp.path().join("known_hosts_1");
    fs::write(&known_hosts_1, format!("[127.0.0.1]:{} {}\n", port1, pub1.trim())).unwrap();

    run_git_ssh_cmd_strict(&init_dir, &key_path, &known_hosts_1, &["init", "-b", "main"]);
    fs::write(init_dir.join("init.txt"), b"initial commit").unwrap();
    run_git_ssh_cmd_strict(&init_dir, &key_path, &known_hosts_1, &["add", "."]);
    run_git_ssh_cmd_strict(&init_dir, &key_path, &known_hosts_1, &["commit", "-m", "Initial commit"]);
    let repo_url_1 = format!("ssh://git@127.0.0.1:{}/restart_user/restart-repo.git", port1);
    run_git_ssh_cmd_strict(&init_dir, &key_path, &known_hosts_1, &["remote", "add", "origin", &repo_url_1]);
    let (ok, out, err) = run_git_ssh_cmd_strict(&init_dir, &key_path, &known_hosts_1, &["push", "-u", "origin", "main"]);
    assert!(ok, "Initial push failed: STDOUT: {}\nSTDERR: {}", out, err);

    // 2. Second Server Boot pointing to the EXACT same storage root and database
    let (addr2, _handle2) = spine_api::start_ssh_server(state.clone(), "127.0.0.1:0".parse().unwrap())
        .await
        .expect("Server 2 boot");
    let port2 = addr2.port();

    let pub2 = fs::read_to_string(&host_pub_path).unwrap();
    // Host key identity MUST be identical across restart (no accidental regeneration)
    assert_eq!(pub1, pub2, "SSH host key MUST persist stably across server restarts");

    // Known hosts with server 2's port
    let known_hosts_2 = tmp.path().join("known_hosts_2");
    fs::write(&known_hosts_2, format!("[127.0.0.1]:{} {}\n", port2, pub2.trim())).unwrap();

    // Client clones from server 2 using existing persisted key
    let repo_url_2 = format!("ssh://git@127.0.0.1:{}/restart_user/restart-repo.git", port2);
    let clone_dir = tmp.path().join("restart_clone");
    let (ok, out, err) = run_git_ssh_cmd_strict(tmp.path(), &key_path, &known_hosts_2, &["clone", &repo_url_2, clone_dir.to_str().unwrap()]);
    assert!(ok, "Clone from server 2 failed:\nSTDOUT: {}\nSTDERR: {}", out, err);
    assert_eq!(fs::read_to_string(clone_dir.join("init.txt")).unwrap(), "initial commit");

    // 3. Key Revocation Test
    db.revoke_user_ssh_key("key_restart", &user.id).await.unwrap();

    // Now push attempt MUST be rejected due to revoked key
    fs::write(clone_dir.join("file.txt"), b"revocation edit").unwrap();
    run_git_ssh_cmd_strict(&clone_dir, &key_path, &known_hosts_2, &["add", "."]);
    run_git_ssh_cmd_strict(&clone_dir, &key_path, &known_hosts_2, &["commit", "-m", "Revoked key commit"]);
    let (ok, out, err) = run_git_ssh_cmd_strict(&clone_dir, &key_path, &known_hosts_2, &["push", "origin", "main"]);
    assert!(!ok, "Push using revoked SSH key must be rejected");
    assert!(err.contains("Permission denied") || err.contains("Could not read") || out.contains("Permission denied"),
        "Expected permission denied on revoked key: STDOUT: {}\nSTDERR: {}", out, err);
}
