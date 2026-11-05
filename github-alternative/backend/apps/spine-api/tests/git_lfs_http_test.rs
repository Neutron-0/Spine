use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use sha2::{Digest, Sha256};
use spine_api::api::{build_router, AppState};
use spine_api::auth::pat::hash_pat;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::fs;
use std::process::Command;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tower::ServiceExt;

fn encode_base64(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    let mut i = 0;
    while i < input.len() {
        let b0 = input[i] as usize;
        let b1 = if i + 1 < input.len() {
            input[i + 1] as usize
        } else {
            0
        };
        let b2 = if i + 2 < input.len() {
            input[i + 2] as usize
        } else {
            0
        };
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(triple >> 18) & 0x3F] as char);
        out.push(CHARS[(triple >> 12) & 0x3F] as char);
        if i + 1 < input.len() {
            out.push(CHARS[(triple >> 6) & 0x3F] as char);
        } else {
            out.push('=');
        }
        if i + 2 < input.len() {
            out.push(CHARS[triple & 0x3F] as char);
        } else {
            out.push('=');
        }
        i += 3;
    }
    out
}

async fn setup_test_server() -> (String, Arc<dyn Database>, AppState, tempfile::TempDir) {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let base_url = format!("http://127.0.0.1:{}", port);

    // Update public_origin on state to match our listening server
    let state = state.with_public_origin(base_url.clone());
    let router = build_router(state.clone());

    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    (base_url, db, state, tmp)
}

fn run_git_cmd(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let mut cmd_args = vec![
        "-c",
        "credential.helper=",
        "-c",
        "core.askPass=echo",
        "-c",
        "core.autocrlf=false",
    ];
    cmd_args.extend_from_slice(args);

    let output = Command::new("git")
        .args(&cmd_args)
        .current_dir(cwd)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_AUTHOR_NAME", "Spine LFS Test")
        .env("GIT_AUTHOR_EMAIL", "lfs_tester@spine.build")
        .env("GIT_COMMITTER_NAME", "Spine LFS Test")
        .env("GIT_COMMITTER_EMAIL", "lfs_tester@spine.build")
        .output()
        .expect("Failed to execute git");

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_lfs_batch_api_conformance() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None)
        .unwrap()
        .with_public_origin("http://127.0.0.1:3000");
    let router = build_router(state.clone());

    // 1. Create User and PAT
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_alice_lfs".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice LFS".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_alice_lfs_token_1234567890abcdef1234";
    let pat = PersonalAccessToken {
        id: "pat_alice_lfs".to_string(),
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

    let _repo = state
        .repositories
        .create_repository(
            &user,
            "lfs-conformance-repo",
            Some("LFS".into()),
            false,
            None,
        )
        .await
        .unwrap();

    let basic_auth_hdr = format!(
        "Basic {}",
        encode_base64(format!("alice:{}", pat_secret).as_bytes())
    );

    // Generate 5 MiB test data (larger than default 4 MiB chunk size)
    let payload_size = 5 * 1024 * 1024; // 5 MiB
    let mut test_bytes = Vec::with_capacity(payload_size);
    for i in 0..payload_size {
        test_bytes.push(((i * 31 + 17) % 256) as u8);
    }
    let mut hasher = Sha256::new();
    hasher.update(&test_bytes);
    let test_oid = hex::encode(hasher.finalize());

    // Case 1: Unsupported transfer adapter -> 422 Unprocessable Entity
    let bad_adapter_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["custom-unsupported-adapter"],
        "objects": [{ "oid": test_oid, "size": payload_size }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&bad_adapter_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Case 2: Unsupported hash algorithm -> 422 Unprocessable Entity
    let bad_hash_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "hash_algo": "sha1",
        "objects": [{ "oid": test_oid, "size": payload_size }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&bad_hash_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);

    // Case 3: Valid upload batch for non-existent object -> returns upload and verify actions
    let upload_batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": test_oid, "size": payload_size }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&upload_batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let body_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let obj_resp = &body_json["objects"][0];
    assert_eq!(obj_resp["oid"], test_oid);
    assert_eq!(obj_resp["size"], payload_size);
    assert!(obj_resp["actions"]["upload"].is_object());
    assert!(obj_resp["actions"]["verify"].is_object());

    let upload_href = obj_resp["actions"]["upload"]["href"].as_str().unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = obj_resp["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    let verify_href = obj_resp["actions"]["verify"]["href"].as_str().unwrap();
    let verify_path = url::Url::parse(verify_href).unwrap().path().to_string();
    let verify_auth = obj_resp["actions"]["verify"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    // Case 4: Execute upload action (PUT) with raw binary payload
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header("Authorization", upload_auth)
        .header("Content-Type", "application/octet-stream")
        .body(Body::from(test_bytes.clone()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Case 5: Execute verify action (POST)
    let verify_body = serde_json::json!({
        "oid": test_oid,
        "size": payload_size
    });
    let req = Request::builder()
        .method("POST")
        .uri(&verify_path)
        .header("Authorization", verify_auth)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&verify_body).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Case 6: Duplicate upload batch -> object already exists, actions MUST be omitted
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&upload_batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let dup_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let dup_json: serde_json::Value = serde_json::from_slice(&dup_bytes).unwrap();
    assert!(
        dup_json["objects"][0]["actions"].is_null(),
        "Existing object must omit actions in upload batch response"
    );

    // Case 7: Download batch for existing object -> returns download action
    let download_batch_req = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": test_oid, "size": payload_size }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&download_batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let dl_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let dl_json: serde_json::Value = serde_json::from_slice(&dl_bytes).unwrap();
    let dl_action = &dl_json["objects"][0]["actions"]["download"];
    let dl_href = dl_action["href"].as_str().unwrap();
    let dl_path = url::Url::parse(dl_href).unwrap().path().to_string();
    let dl_auth = dl_action["header"]["Authorization"].as_str().unwrap();

    // Case 8: Execute download action (GET) -> returns exact raw bytes
    let req = Request::builder()
        .method("GET")
        .uri(&dl_path)
        .header("Authorization", dl_auth)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(downloaded_bytes.len(), payload_size);
    assert_eq!(&downloaded_bytes[..], &test_bytes[..]);

    // Case 9: Download batch for non-existent object -> per-object 404 error
    let fake_oid = "0000000000000000000000000000000000000000000000000000000000000000";
    let fake_dl_req = serde_json::json!({
        "operation": "download",
        "objects": [{ "oid": fake_oid, "size": 100 }]
    });
    let req = Request::builder()
        .method("POST")
        .uri("/alice/lfs-conformance-repo.git/info/lfs/objects/batch")
        .header("Authorization", &basic_auth_hdr)
        .header("Content-Type", "application/vnd.git-lfs+json")
        .body(Body::from(serde_json::to_vec(&fake_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let fake_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let fake_json: serde_json::Value = serde_json::from_slice(&fake_bytes).unwrap();
    assert_eq!(fake_json["objects"][0]["error"]["code"], 404);

    // Case 10: Security check - tampered token rejected
    let tampered_auth = format!("{}tampered", upload_auth);
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header("Authorization", tampered_auth)
        .body(Body::from(vec![0u8; 10]))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_git_lfs_receive_pack_pre_receive_rejection() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_bob_lfs".to_string(),
        username: "bob".to_string(),
        normalized_username: "bob".to_string(),
        email: "bob@spine.build".to_string(),
        normalized_email: "bob@spine.build".to_string(),
        display_name: "Bob LFS".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_bob_lfs_token_1234567890abcdef1234";
    let pat = PersonalAccessToken {
        id: "pat_bob_lfs".to_string(),
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

    let repo = state
        .repositories
        .create_repository(
            &user,
            "lfs-reject-repo",
            Some("LFS Reject".into()),
            false,
            None,
        )
        .await
        .unwrap();

    let port = url::Url::parse(&base_url).unwrap().port().unwrap();
    let auth_remote = format!(
        "http://bob:{}@127.0.0.1:{}/bob/{}.git",
        pat_secret, port, repo.name
    );

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    run_git_cmd(&local_repo, &["init", "-b", "main"]);

    // Create a commit with a manual Git LFS pointer pointing to a 10 MiB object that is NEVER uploaded!
    let fake_lfs_oid = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";
    let fake_pointer = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize 10485760\n",
        fake_lfs_oid
    );
    fs::write(local_repo.join("missing_asset.bin"), fake_pointer).unwrap();
    run_git_cmd(&local_repo, &["add", "missing_asset.bin"]);
    run_git_cmd(
        &local_repo,
        &["commit", "-m", "Commit with missing LFS pointer"],
    );

    // Push should be rejected by pre-receive verification
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", &auth_remote, "main"]);
    assert!(
        !ok,
        "Pushing commit with missing LFS object should fail! stdout: {}, stderr: {}",
        stdout, stderr
    );
    assert!(
        stderr.contains("missing") && stderr.contains(fake_lfs_oid),
        "Error message should mention missing LFS object: {}",
        stderr
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_official_git_lfs_workflow() {
    let (base_url, db, state, _tmp) = setup_test_server().await;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_carol_lfs".to_string(),
        username: "carol".to_string(),
        normalized_username: "carol".to_string(),
        email: "carol@spine.build".to_string(),
        normalized_email: "carol@spine.build".to_string(),
        display_name: "Carol LFS".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_carol_lfs_token_1234567890abcdef1234";
    let pat = PersonalAccessToken {
        id: "pat_carol_lfs".to_string(),
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
        .create_repository(
            &user,
            "official-lfs-repo",
            Some("Official LFS".into()),
            false,
            None,
        )
        .await
        .unwrap();

    let port = url::Url::parse(&base_url).unwrap().port().unwrap();
    let auth_remote = format!(
        "http://carol:{}@127.0.0.1:{}/carol/{}.git",
        pat_secret, port, repo.name
    );

    let client_tmp = tempdir().unwrap();
    let local_repo = client_tmp.path().join("local_repo");
    fs::create_dir_all(&local_repo).unwrap();

    // 1. Initialize Git repository with Git LFS
    run_git_cmd(&local_repo, &["init", "-b", "main"]);
    run_git_cmd(&local_repo, &["lfs", "install", "--local"]);
    run_git_cmd(&local_repo, &["lfs", "track", "*.bin"]);
    run_git_cmd(&local_repo, &["add", ".gitattributes"]);

    // 2. Generate a 5 MiB binary asset (exceeding default 4 MiB chunk threshold)
    let payload_size = 5 * 1024 * 1024;
    let mut initial_binary = Vec::with_capacity(payload_size);
    for i in 0..payload_size {
        initial_binary.push(((i * 73 + 19) % 256) as u8);
    }
    fs::write(local_repo.join("model_weights.bin"), &initial_binary).unwrap();

    run_git_cmd(&local_repo, &["add", "model_weights.bin"]);
    run_git_cmd(
        &local_repo,
        &["commit", "-m", "Add 5 MiB model weights via LFS"],
    );

    // Verify pointer was committed into local git object database
    let (ok, cat_out, _) = run_git_cmd(&local_repo, &["cat-file", "-p", "HEAD:model_weights.bin"]);
    assert!(ok);
    assert!(
        cat_out.starts_with("version https://git-lfs.github.com/spec/v1"),
        "Git tree must store the LFS pointer text: {}",
        cat_out
    );

    // 3. Push to Spine Forge remote using official Git CLI + Git LFS
    run_git_cmd(&local_repo, &["remote", "add", "origin", &auth_remote]);
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", "-u", "origin", "main"]);
    assert!(
        ok,
        "Official git push with LFS failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // 4. Clone into a fresh directory using official Git CLI + Git LFS
    let clone_dir = client_tmp.path().join("fresh_clone");
    let (ok, stdout, stderr) = run_git_cmd(
        client_tmp.path(),
        &["clone", &auth_remote, clone_dir.to_str().unwrap()],
    );
    assert!(
        ok,
        "Official git clone with LFS failed. stdout: {}, stderr: {}",
        stdout, stderr
    );

    // 5. Verify the reconstructed binary file is hydrated byte-for-byte!
    let cloned_binary_path = clone_dir.join("model_weights.bin");
    assert!(cloned_binary_path.exists());
    let cloned_binary = fs::read(&cloned_binary_path).unwrap();
    assert_eq!(cloned_binary.len(), payload_size);
    assert_eq!(
        cloned_binary, initial_binary,
        "Hydrated LFS binary in fresh clone must match initial binary byte-for-byte"
    );

    // 6. Modify the binary, commit and push
    let mut modified_binary = initial_binary.clone();
    modified_binary.extend_from_slice(b"ADDITIONAL_DATA_APPENDED_TO_LFS_OBJECT_12345");
    fs::write(local_repo.join("model_weights.bin"), &modified_binary).unwrap();
    run_git_cmd(&local_repo, &["add", "model_weights.bin"]);
    run_git_cmd(&local_repo, &["commit", "-m", "Update model weights"]);
    let (ok, stdout, stderr) = run_git_cmd(&local_repo, &["push", "origin", "main"]);
    assert!(ok, "Second push failed: {} {}", stdout, stderr);

    // 7. Pull into fresh_clone and verify updated binary
    let (ok, stdout, stderr) = run_git_cmd(&clone_dir, &["pull", "origin", "main"]);
    assert!(ok, "Git pull failed: {} {}", stdout, stderr);

    let updated_cloned_binary = fs::read(&cloned_binary_path).unwrap();
    assert_eq!(updated_cloned_binary.len(), modified_binary.len());
    assert_eq!(
        updated_cloned_binary, modified_binary,
        "Updated LFS binary after pull must match modified binary byte-for-byte"
    );
}
