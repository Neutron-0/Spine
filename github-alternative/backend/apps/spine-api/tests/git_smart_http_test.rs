use spine_api::api::{build_router, AppState};
use spine_api::auth::pat::hash_pat;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::fs;
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;

async fn setup_test_server() -> (String, Arc<dyn Database>, AppState, tempfile::TempDir) {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();
    let router = build_router(state.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    (base_url, db, state, tmp)
}

fn run_git_cmd(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let mut cmd_args = vec![
        "-c", "credential.helper=",
        "-c", "core.askPass=echo",
        "-c", "core.autocrlf=false",
    ];
    cmd_args.extend_from_slice(args);

    let output = Command::new("git")
        .args(&cmd_args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_AUTHOR_NAME", "Spine Test")
        .env("GIT_AUTHOR_EMAIL", "tester@spine.build")
        .env("GIT_COMMITTER_NAME", "Spine Test")
        .env("GIT_COMMITTER_EMAIL", "tester@spine.build")
        .output()
        .expect("Failed to execute git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_smart_http_push_clone_roundtrip() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    // 1. Create User and PAT
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_alice_token_1234567890abcdef12345678";
    let pat = PersonalAccessToken {
        id: "pat_alice".to_string(),
        user_id: user.id.clone(),
        name: "Alice PAT".to_string(),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    // 2. Create public repository
    let repo = state
        .repositories
        .create_repository(&user, "demo-project", Some("Demo".into()), false, None)
        .await
        .unwrap();

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    // Init local git repo
    let (ok, _, err) = run_git_cmd(&local_repo, &["init", "-b", "main"]);
    assert!(ok, "git init failed: {}", err);

    let readme = local_repo.join("README.md");
    fs::write(&readme, "# Hello Spine\nThis is native Git Smart HTTP.\n").unwrap();

    let (ok, _, err) = run_git_cmd(&local_repo, &["add", "README.md"]);
    assert!(ok, "git add failed: {}", err);

    let (ok, _, err) = run_git_cmd(&local_repo, &["commit", "-m", "Initial commit"]);
    assert!(ok, "git commit failed: {}", err);

    // Push using PAT credentials
    let remote_url = format!("{}/alice/{}.git", base_url, repo.name);
    let auth_remote_url = format!(
        "http://alice:{}@127.0.0.1:{}/alice/{}.git",
        pat_secret,
        url::Url::parse(&base_url).unwrap().port().unwrap(),
        repo.name
    );

    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", &auth_remote_url, "main"]);
    assert!(
        ok,
        "git push failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // 3. Clone repository into a second directory anonymously (public repo)
    let clone_dir = client_tmp.path().join("cloned_repo");
    let (ok, stdout, stderr) = run_git_cmd(
        client_tmp.path(),
        &["clone", &remote_url, clone_dir.to_str().unwrap()],
    );
    assert!(
        ok,
        "git clone failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    let cloned_readme = clone_dir.join("README.md");
    assert!(cloned_readme.exists(), "README.md missing after clone");
    let content = fs::read_to_string(cloned_readme).unwrap();
    assert_eq!(content.replace("\r\n", "\n"), "# Hello Spine\nThis is native Git Smart HTTP.\n");

    // 4. Test incremental push from cloned repository
    let license = clone_dir.join("LICENSE");
    fs::write(&license, "Apache 2.0\n").unwrap();
    let (ok, _, err) = run_git_cmd(&clone_dir, &["add", "LICENSE"]);
    assert!(ok, "git add license failed: {}", err);
    let (ok, _, err) = run_git_cmd(&clone_dir, &["commit", "-m", "Add LICENSE"]);
    assert!(ok, "git commit license failed: {}", err);

    let (ok, stdout, stderr) = run_git_cmd(&clone_dir, &["push", &auth_remote_url, "main"]);
    assert!(
        ok,
        "second git push failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // 5. In original repo, test pull
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["pull", &remote_url, "main"]);
    assert!(
        ok,
        "git pull failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(local_repo.join("LICENSE").exists(), "LICENSE missing after git pull");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_smart_http_private_repository_authentication() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_bob".to_string(),
        username: "bob".to_string(),
        normalized_username: "bob".to_string(),
        email: "bob@spine.build".to_string(),
        normalized_email: "bob@spine.build".to_string(),
        display_name: "Bob Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_bob_token_1234567890abcdef1234567890";
    let pat = PersonalAccessToken {
        id: "pat_bob".to_string(),
        user_id: user.id.clone(),
        name: "Bob PAT".to_string(),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    // Create private repository
    let repo = state
        .repositories
        .create_repository(&user, "secret-project", Some("Secret".into()), true, None)
        .await
        .unwrap();

    let client_tmp = tempdir().unwrap();
    let port = url::Url::parse(&base_url).unwrap().port().unwrap();

    // 1. Anonymous clone must FAIL with authentication prompt / 401
    let unauth_url = format!("{}/bob/{}.git", base_url, repo.name);
    let fail_clone = client_tmp.path().join("fail_clone");
    let (ok, _stdout, stderr) = run_git_cmd(
        client_tmp.path(),
        &["clone", &unauth_url, fail_clone.to_str().unwrap()],
    );
    assert!(!ok, "Anonymous clone of private repository should fail!");
    assert!(
        stderr.contains("Authentication required")
            || stderr.contains("terminal prompts disabled")
            || stderr.contains("401")
            || stderr.contains("not found"),
        "Expected auth failure in stderr: {}",
        stderr
    );

    // 2. Authenticated clone with PAT must SUCCEED
    let auth_url = format!(
        "http://bob:{}@127.0.0.1:{}/bob/{}.git",
        pat_secret, port, repo.name
    );
    let success_clone = client_tmp.path().join("success_clone");
    let (ok, stdout, stderr) = run_git_cmd(
        client_tmp.path(),
        &["clone", &auth_url, success_clone.to_str().unwrap()],
    );
    assert!(
        ok,
        "Authenticated clone failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_smart_http_branch_protection_rejection() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_carol".to_string(),
        username: "carol".to_string(),
        normalized_username: "carol".to_string(),
        email: "carol@spine.build".to_string(),
        normalized_email: "carol@spine.build".to_string(),
        display_name: "Carol Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_carol_token_1234567890abcdef12345678";
    let pat = PersonalAccessToken {
        id: "pat_carol".to_string(),
        user_id: user.id.clone(),
        name: "Carol PAT".to_string(),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    let repo = state
        .repositories
        .create_repository(&user, "protected-repo", Some("Protected".into()), false, None)
        .await
        .unwrap();

    // Add branch protection rule requiring pull request on main
    let rule = BranchProtectionRule {
        id: "bpr_main".to_string(),
        repo_id: repo.id.clone(),
        branch_pattern: "main".to_string(),
        require_pull_request: true,
        required_approvals: 1,
        prevent_self_approval: true,
        require_changes_addressed: true,
        dismiss_stale_reviews_on_push: false,
        restrict_merges_to_admins: false,
        required_status_checks: vec![],
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.set_branch_protection(rule).await.unwrap();

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    run_git_cmd(&local_repo, &["init", "-b", "main"]);
    fs::write(local_repo.join("file.txt"), "Protected branch text\n").unwrap();
    run_git_cmd(&local_repo, &["add", "file.txt"]);
    run_git_cmd(&local_repo, &["commit", "-m", "Direct push to protected branch"]);

    let port = url::Url::parse(&base_url).unwrap().port().unwrap();
    let auth_remote = format!(
        "http://carol:{}@127.0.0.1:{}/carol/{}.git",
        pat_secret, port, repo.name
    );

    // Push directly to protected main branch: MUST BE REJECTED
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", &auth_remote, "main"]);
    assert!(!ok, "Push to protected branch should have been rejected!");
    assert!(
        stderr.contains("protected branch") || stderr.contains("pull request required"),
        "Expected branch protection rejection message. Got stderr: {}, stdout: {}",
        stderr,
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_smart_http_secret_scanning_rejection() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_dave".to_string(),
        username: "dave".to_string(),
        normalized_username: "dave".to_string(),
        email: "dave@spine.build".to_string(),
        normalized_email: "dave@spine.build".to_string(),
        display_name: "Dave Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_dave_token_1234567890abcdef12345678";
    let pat = PersonalAccessToken {
        id: "pat_dave".to_string(),
        user_id: user.id.clone(),
        name: "Dave PAT".to_string(),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    let repo = state
        .repositories
        .create_repository(&user, "scanner-repo", Some("Scanner".into()), false, None)
        .await
        .unwrap();

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    run_git_cmd(&local_repo, &["init", "-b", "dev"]);
    // Introduce a secret token into a file
    let secret_file = local_repo.join("credentials.env");
    fs::write(
        &secret_file,
        "SPINE_TOKEN=sp_pat_leaked_secret_token_1234567890abcdef12345\n",
    )
    .unwrap();
    run_git_cmd(&local_repo, &["add", "credentials.env"]);
    run_git_cmd(&local_repo, &["commit", "-m", "Accidental secret commit"]);

    let port = url::Url::parse(&base_url).unwrap().port().unwrap();
    let auth_remote = format!(
        "http://dave:{}@127.0.0.1:{}/dave/{}.git",
        pat_secret, port, repo.name
    );

    // Push commit with leaked secret: MUST BE REJECTED by secret scanner
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", &auth_remote, "dev"]);
    assert!(!ok, "Push containing secret should have been rejected!");
    assert!(
        stderr.contains("secret scanning") || stderr.contains("sensitive credentials"),
        "Expected secret scanning rejection message. Got stderr: {}, stdout: {}",
        stderr,
        stdout
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_smart_http_url_without_dot_git_and_tags() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_eve".to_string(),
        username: "eve".to_string(),
        normalized_username: "eve".to_string(),
        email: "eve@spine.build".to_string(),
        normalized_email: "eve@spine.build".to_string(),
        display_name: "Eve Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_eve_token_1234567890abcdef1234567890";
    let pat = PersonalAccessToken {
        id: "pat_eve".to_string(),
        user_id: user.id.clone(),
        name: "Eve PAT".to_string(),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();

    let repo = state
        .repositories
        .create_repository(&user, "tag-project", Some("Tag Test".into()), false, None)
        .await
        .unwrap();

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    run_git_cmd(&local_repo, &["init", "-b", "main"]);
    fs::write(local_repo.join("version.txt"), "1.0.0\n").unwrap();
    run_git_cmd(&local_repo, &["add", "version.txt"]);
    run_git_cmd(&local_repo, &["commit", "-m", "Release 1.0.0"]);
    run_git_cmd(&local_repo, &["tag", "-a", "v1.0.0", "-m", "Version 1.0.0"]);

    let port = url::Url::parse(&base_url).unwrap().port().unwrap();
    // Test URL WITHOUT .git suffix: http://eve:pat@127.0.0.1:port/eve/tag-project
    let auth_remote_no_git = format!(
        "http://eve:{}@127.0.0.1:{}/eve/{}",
        pat_secret, port, repo.name
    );

    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", &auth_remote_no_git, "main", "--tags"]);
    assert!(
        ok,
        "Push without .git suffix failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // Clone without .git suffix anonymously
    let unauth_no_git = format!("{}/eve/{}", base_url, repo.name);
    let clone_dir = client_tmp.path().join("clone_no_git");
    let (ok, stdout, stderr) = run_git_cmd(
        client_tmp.path(),
        &["clone", &unauth_no_git, clone_dir.to_str().unwrap()],
    );
    assert!(
        ok,
        "Clone without .git suffix failed. stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(clone_dir.join("version.txt").exists());
}
