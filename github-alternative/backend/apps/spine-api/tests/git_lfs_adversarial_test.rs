use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use sha2::{Digest, Sha256};
use spine_api::api::routes_lfs::{generate_scoped_token, LFS_MEDIA_TYPE};
use spine_api::api::{build_router, AppState};
use spine_api::auth::pat::hash_pat;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_artifact::ArtifactGcCollector;
use std::fs;
use std::sync::Arc;
use tempfile::tempdir;
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

async fn setup_test_env() -> (
    Arc<dyn Database>,
    AppState,
    axum::Router,
    User,
    String,
    ForgeRepository,
) {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None)
        .unwrap()
        .with_public_origin("http://127.0.0.1:3000");
    let router = build_router(state.clone());

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "usr_alice_adv".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice Adversarial".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat_secret = "sp_pat_test12345".to_string();
    let pat = PersonalAccessToken {
        id: "pat_alice_adv".to_string(),
        user_id: user.id.clone(),
        name: "Alice Adversarial PAT".to_string(),
        token_hash: hash_pat(&pat_secret),
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
            "adv-lfs-repo",
            Some("Adversarial LFS Repo".into()),
            false,
            None,
        )
        .await
        .unwrap();

    (db, state, router, user, pat_secret, repo)
}

async fn create_test_user_and_pat(
    db: &Arc<dyn Database>,
    id: &str,
    username: &str,
    email: &str,
    pat_secret: &str,
) -> (User, String) {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: id.to_string(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: email.to_string(),
        normalized_email: email.to_lowercase(),
        display_name: username.to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let pat = PersonalAccessToken {
        id: format!("pat_{}", username),
        user_id: user.id.clone(),
        name: format!("{} PAT", username),
        token_hash: hash_pat(pat_secret),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: i64::MAX,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };
    db.create_pat(pat).await.unwrap();
    (user, pat_secret.to_string())
}

#[tokio::test]
async fn test_adversarial_lfs_hash_mismatch_and_no_manifest_persisted() {
    let (db, _state, router, user, pat_secret, repo) = setup_test_env().await;

    let basic_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", user.username, pat_secret).as_bytes())
    );

    let legitimate_bytes = b"LEGITIMATE_EXPECTED_PAYLOAD_DATA_BYTES";
    let mut legitimate_hasher = Sha256::new();
    legitimate_hasher.update(legitimate_bytes);
    let legitimate_oid = hex::encode(legitimate_hasher.finalize());

    // 1. Batch request declaring legitimate_oid
    let batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": legitimate_oid, "size": legitimate_bytes.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, &basic_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let upload_href = batch_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = batch_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    // 2. Adversarial upload: send mismatched body bytes!
    let corrupt_bytes = b"CORRUPTED_ATTACKER_SUPPLIED_BYTES_DIFFERENT_HASH";
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(corrupt_bytes.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let err_json: serde_json::Value = serde_json::from_slice(&err_bytes).unwrap();
    assert!(
        err_json["message"]
            .as_str()
            .unwrap()
            .contains("Payload SHA-256 mismatch"),
        "Error message must indicate SHA-256 mismatch: {:?}",
        err_json
    );

    // 3. Confirm invariant: no manifest was persisted into the database!
    let found_by_oid = db
        .get_artifact_manifest_by_repo_oid(&repo.id, &legitimate_oid)
        .await
        .unwrap();
    assert!(
        found_by_oid.is_none(),
        "Mismatched upload must NEVER publish a manifest into the database!"
    );

    let found_by_id = db.get_artifact_manifest(&legitimate_oid).await.unwrap();
    assert!(
        found_by_id.is_none(),
        "Mismatched upload must not publish an artifact under legitimate OID"
    );

    // 4. Verify endpoint also returns 404
    let verify_href = batch_json["objects"][0]["actions"]["verify"]["href"]
        .as_str()
        .unwrap();
    let verify_path = url::Url::parse(verify_href).unwrap().path().to_string();
    let verify_auth = batch_json["objects"][0]["actions"]["verify"]["header"]["Authorization"]
        .as_str()
        .unwrap();
    let verify_body = serde_json::json!({
        "oid": legitimate_oid,
        "size": legitimate_bytes.len()
    });
    let req = Request::builder()
        .method("POST")
        .uri(&verify_path)
        .header(header::AUTHORIZATION, verify_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&verify_body).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn test_adversarial_lfs_content_length_mismatch() {
    let (db, _state, router, user, pat_secret, repo) = setup_test_env().await;

    let basic_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", user.username, pat_secret).as_bytes())
    );

    let test_bytes = b"CORRECT_HASH_BYTES_FOR_SIZE_MISMATCH";
    let mut hasher = Sha256::new();
    hasher.update(test_bytes);
    let oid = hex::encode(hasher.finalize());

    // Batch request
    let batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": oid, "size": test_bytes.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, &basic_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let upload_href = batch_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = batch_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    // Adversarial upload: declare Content-Length as 50000, but only send actual test_bytes.len() (36)
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, "50000")
        .body(Body::from(test_bytes.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    let err_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let err_json: serde_json::Value = serde_json::from_slice(&err_bytes).unwrap();
    assert!(
        err_json["message"]
            .as_str()
            .unwrap()
            .contains("Payload size mismatch"),
        "Error message must indicate size mismatch: {:?}",
        err_json
    );

    // Confirm no manifest persisted
    let found = db
        .get_artifact_manifest_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap();
    assert!(found.is_none());
}

#[tokio::test]
async fn test_adversarial_lfs_scoped_token_tampering_and_forgery() {
    let (_db, _state, router, user, _pat_secret, repo) = setup_test_env().await;

    let oid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let (valid_token, _exp) = generate_scoped_token(&repo.id, oid, "upload", 1024 * 1024, 3600);

    let upload_uri = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo.name, oid
    );

    // Case 1: Missing auth header -> 401 Unauthorized
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // Case 2: Malformed token format (no dot separator) -> 403 Forbidden
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri)
        .header(header::AUTHORIZATION, "Bearer not_a_valid_token_string")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Case 3: Tampered signature -> 403 Forbidden
    let (payload, _sig) = valid_token.split_once('.').unwrap();
    let forged_token = format!(
        "{}.0000000000000000000000000000000000000000000000000000000000000000",
        payload
    );
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri)
        .header(header::AUTHORIZATION, format!("Bearer {}", forged_token))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // Case 4: Expired token -> 403 Forbidden
    let (expired_token, _exp) = generate_scoped_token(&repo.id, oid, "upload", 1024 * 1024, -100);
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri)
        .header(header::AUTHORIZATION, format!("Bearer {}", expired_token))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_adversarial_lfs_token_scope_confusion_and_cross_repo_reuse() {
    let (_db, state, router, user, _pat_secret, repo_a) = setup_test_env().await;

    // Create a second repo B owned by user
    let repo_b = state
        .repositories
        .create_repository(&user, "adv-lfs-repo-b", Some("Repo B".into()), false, None)
        .await
        .unwrap();

    let oid_a = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let oid_b = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    // Mint scoped tokens
    let (upload_token_a, _) = generate_scoped_token(&repo_a.id, oid_a, "upload", 1024 * 1024, 3600);
    let (download_token_a, _) =
        generate_scoped_token(&repo_a.id, oid_a, "download", 1024 * 1024, 3600);
    let (verify_token_a, _) = generate_scoped_token(&repo_a.id, oid_a, "verify", 1024 * 1024, 3600);

    let upload_uri_a = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo_a.name, oid_a
    );
    let upload_uri_b = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo_b.name, oid_a
    );
    let download_uri_a = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo_a.name, oid_a
    );
    let verify_uri_a = format!("/{}/{}.git/info/lfs/verify", user.username, repo_a.name);

    // 1. Cross-repo reuse: Upload token for repo A used against repo B -> 403 Forbidden
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri_b)
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 2. Cross-OID reuse: Upload token for oid_a used to upload oid_b -> 403 Forbidden
    let upload_uri_wrong_oid = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo_a.name, oid_b
    );
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri_wrong_oid)
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 3. Scope confusion: upload token used on download endpoint -> 403 Forbidden
    let req = Request::builder()
        .method("GET")
        .uri(&download_uri_a)
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 4. Scope confusion: download token used on upload endpoint -> 403 Forbidden
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri_a)
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", download_token_a),
        )
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 5. Scope confusion: verify token used on upload endpoint -> 403 Forbidden
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_uri_a)
        .header(header::AUTHORIZATION, format!("Bearer {}", verify_token_a))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 6. Scope confusion: upload token used on verify endpoint -> 403 Forbidden
    let verify_payload = serde_json::json!({
        "oid": oid_a,
        "size": 1024
    });
    let req = Request::builder()
        .method("POST")
        .uri(&verify_uri_a)
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&verify_payload).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_adversarial_lfs_deduplication_and_duplicate_upload() {
    let (db, _state, router, user, pat_secret, repo) = setup_test_env().await;

    let basic_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", user.username, pat_secret).as_bytes())
    );

    let payload = b"DEDUPLICATED_LFS_OBJECT_CONTENT_REPEATED";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // 1. First upload
    let batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": oid, "size": payload.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, &basic_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let upload_href = batch_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = batch_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify record exists in DB and OID resolves
    let manifest1 = db
        .get_artifact_manifest_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap()
        .expect("Manifest must exist");
    assert_eq!(manifest1.artifact_id, oid);

    // 2. Second batch request for same object -> actions must be omitted!
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, &basic_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json2: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        batch_json2["objects"][0]["actions"].is_null(),
        "Existing object must omit actions in upload batch response: {:?}",
        batch_json2
    );

    // 3. If client re-uploads same object with valid token (e.g. concurrent upload or retry) -> idempotent success
    let (reupload_token, _) =
        generate_scoped_token(&repo.id, &oid, "upload", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, format!("Bearer {}", reupload_token))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Manifest is still cleanly intact
    let manifest2 = db
        .get_artifact_manifest_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap()
        .expect("Manifest must exist");
    assert_eq!(manifest2.artifact_id, oid);
}

#[tokio::test]
async fn test_unrelated_routes_retain_strict_body_limit() {
    let (_db, _state, router, user, _pat_secret, repo) = setup_test_env().await;

    // Root router enforces DefaultBodyLimit::max(10 * 1024 * 1024) (10 MiB)
    // Send 11 MiB payload to an unrelated standard route: POST /api/v1/auth/login
    let large_body = vec![0x41u8; 11 * 1024 * 1024]; // 11 MiB
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/auth/login")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(large_body))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "Unrelated route must reject payload > 10 MiB with 413 Payload Too Large"
    );

    // Check that LFS objects batch route also rejects body > 10 MiB
    let large_lfs_batch = vec![0x20u8; 11 * 1024 * 1024]; // 11 MiB
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(large_lfs_batch))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::PAYLOAD_TOO_LARGE,
        "LFS batch route must reject payload > 10 MiB with 413 Payload Too Large"
    );
}

#[tokio::test]
async fn test_multitenant_lfs_repository_isolation_and_existence_leak_prevention() {
    let (db, state, router, alice, alice_pat, _default_repo) = setup_test_env().await;

    // 1. Create private repo for Alice
    let repo_a = state
        .repositories
        .create_repository(
            &alice,
            "private-repo-a",
            Some("Alice Private Repo".into()),
            true, // private
            None,
        )
        .await
        .unwrap();

    // 2. Create Bob and private repo for Bob
    let (bob, bob_pat) = create_test_user_and_pat(
        &db,
        "usr_bob_adv",
        "bob",
        "bob@spine.build",
        "sp_pat_bob_12345",
    )
    .await;

    let repo_b = state
        .repositories
        .create_repository(
            &bob,
            "private-repo-b",
            Some("Bob Private Repo".into()),
            true, // private
            None,
        )
        .await
        .unwrap();

    let alice_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", alice.username, alice_pat).as_bytes())
    );
    let bob_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", bob.username, bob_pat).as_bytes())
    );

    let payload = b"CONFIDENTIAL_ALICE_PROPRIETARY_PAYLOAD";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // 3. Alice uploads payload to repo_a
    let batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": &oid, "size": payload.len() }]
    });

    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            alice.username, repo_a.name
        ))
        .header(header::AUTHORIZATION, &alice_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

    let upload_href = batch_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = batch_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify object is mapped in repo_a, but NOT in repo_b
    let obj_a = db
        .get_lfs_object_by_repo_oid(&repo_a.id, &oid)
        .await
        .unwrap();
    assert!(obj_a.is_some(), "Object must exist in repo_a");
    let obj_b = db
        .get_lfs_object_by_repo_oid(&repo_b.id, &oid)
        .await
        .unwrap();
    assert!(obj_b.is_none(), "Object must NOT exist in repo_b");

    // 4. Bob queries batch download on repo_b for Alice's OID -> 404 error
    let batch_dl_req = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": &oid, "size": payload.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            bob.username, repo_b.name
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let bob_dl_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        bob_dl_json["objects"][0]["error"]["code"], 404,
        "Bob must receive 404 for unmapped object on repo_b"
    );
    assert!(
        bob_dl_json["objects"][0]["actions"].is_null(),
        "Bob must not receive download actions for unmapped object"
    );

    // 5. CRITICAL: Bob queries batch upload on repo_b for Alice's OID -> MUST return upload action!
    // If actions were omitted, Bob would learn that Alice already uploaded this OID somewhere else in the multi-tenant system!
    let batch_ul_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": &oid, "size": payload.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            bob.username, repo_b.name
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_ul_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let bob_ul_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        !bob_ul_json["objects"][0]["actions"]["upload"].is_null(),
        "Upload batch for unmapped OID on repo_b must return upload action (no cross-repo existence leak)"
    );

    // 6. Direct download on repo_b for Alice's OID -> 404 Not Found
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            bob.username, repo_b.name, oid
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 7. Direct verify on repo_b for Alice's OID -> 404 Not Found
    let verify_req = serde_json::json!({ "oid": &oid, "size": payload.len() });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/verify",
            bob.username, repo_b.name
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&verify_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // 8. Bob attempts direct access to Alice's private repo_a -> 404 / 403
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            alice.username, repo_a.name
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Bob must be rejected when attempting to query Alice's private repository"
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            alice.username, repo_a.name, oid
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Bob must be rejected when attempting to download directly from Alice's private repository"
    );

    // 9. Bob legitimately uploads the same payload to repo_b (deduplication happens in CAS)
    let bob_upload_href = bob_ul_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let bob_upload_path = url::Url::parse(bob_upload_href).unwrap().path().to_string();
    let bob_upload_auth = bob_ul_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri(&bob_upload_path)
        .header(header::AUTHORIZATION, bob_upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Check that both repos now map the object, sharing the deduplicated underlying CAS chunks
    let lfs_a = db
        .get_lfs_object_by_repo_oid(&repo_a.id, &oid)
        .await
        .unwrap()
        .expect("repo_a mapping exists");
    let lfs_b = db
        .get_lfs_object_by_repo_oid(&repo_b.id, &oid)
        .await
        .unwrap()
        .expect("repo_b mapping exists");

    let manifest_a = db
        .get_artifact_manifest(&lfs_a.manifest_id)
        .await
        .unwrap()
        .expect("manifest_a exists");
    let manifest_b = db
        .get_artifact_manifest(&lfs_b.manifest_id)
        .await
        .unwrap()
        .expect("manifest_b exists");
    assert_eq!(
        manifest_a.whole_object_hash, manifest_b.whole_object_hash,
        "Underlying whole object hash must match across repositories"
    );
    assert_eq!(manifest_a.total_size, manifest_b.total_size);

    let store_manifest_a = state
        .artifacts
        .store()
        .get_manifest(&lfs_a.manifest_id)
        .unwrap()
        .expect("store manifest a");
    let store_manifest_b = state
        .artifacts
        .store()
        .get_manifest(&lfs_b.manifest_id)
        .unwrap()
        .expect("store manifest b");
    assert_eq!(
        store_manifest_a.chunks[0].chunk_hash, store_manifest_b.chunks[0].chunk_hash,
        "Underlying CAS chunk hashes must be identical and shared"
    );

    // Bob can now query download on repo_b and receive download action
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            bob.username, repo_b.name
        ))
        .header(header::AUTHORIZATION, &bob_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let bob_dl_json2: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let bob_dl_href = bob_dl_json2["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .unwrap();
    let bob_dl_path = url::Url::parse(bob_dl_href).unwrap().path().to_string();
    let bob_dl_auth = bob_dl_json2["objects"][0]["actions"]["download"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    // Bob downloads from repo_b
    let req = Request::builder()
        .method("GET")
        .uri(&bob_dl_path)
        .header(header::AUTHORIZATION, bob_dl_auth)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded_bytes[..], payload);

    // 10. Collaborator and Revocation: Carol (User 3)
    let (carol, carol_pat) = create_test_user_and_pat(
        &db,
        "usr_carol_adv",
        "carol",
        "carol@spine.build",
        "sp_pat_carol_12345",
    )
    .await;
    let carol_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", carol.username, carol_pat).as_bytes())
    );

    // Carol tries to access Alice's private repo_a without permission -> rejected
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            alice.username, repo_a.name
        ))
        .header(header::AUTHORIZATION, &carol_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Carol without permission must be rejected from repo_a"
    );

    // Alice grants Carol Reader access
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    db.create_permission(RepositoryPermission {
        id: "perm_carol_reader".to_string(),
        repo_id: repo_a.id.clone(),
        user_id: carol.id.clone(),
        role: PermissionRole::Reader,
        created_at_nanos: now,
    })
    .await
    .unwrap();

    // Carol now successfully queries batch download on repo_a
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            alice.username, repo_a.name
        ))
        .header(header::AUTHORIZATION, &carol_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let carol_dl_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    let carol_dl_href = carol_dl_json["objects"][0]["actions"]["download"]["href"]
        .as_str()
        .unwrap();
    let carol_dl_path = url::Url::parse(carol_dl_href).unwrap().path().to_string();
    let carol_dl_auth = carol_dl_json["objects"][0]["actions"]["download"]["header"]
        ["Authorization"]
        .as_str()
        .unwrap();

    // Carol downloads from repo_a
    let req = Request::builder()
        .method("GET")
        .uri(&carol_dl_path)
        .header(header::AUTHORIZATION, carol_dl_auth)
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let carol_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&carol_bytes[..], payload);

    // Alice revokes Carol's permission
    db.delete_permission(&repo_a.id, &carol.id).await.unwrap();

    // Carol's access is immediately revoked
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            alice.username, repo_a.name
        ))
        .header(header::AUTHORIZATION, &carol_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_dl_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert!(
        resp.status() == StatusCode::NOT_FOUND || resp.status() == StatusCode::FORBIDDEN,
        "Revoked collaborator must be rejected immediately"
    );
}

#[tokio::test]
async fn test_concurrent_duplicate_lfs_uploads() {
    let (db, _state, router, user, _pat_secret, repo) = setup_test_env().await;

    let payload = b"CONCURRENT_DUPLICATE_LFS_UPLOAD_TEST_PAYLOAD";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // Generate valid scoped upload token for each concurrent task
    let mut handles = Vec::new();
    for _ in 0..5 {
        let (token, _) =
            generate_scoped_token(&repo.id, &oid, "upload", payload.len() as u64, 3600);
        let upload_path = format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        );
        let router_clone = router.clone();
        let payload_vec = payload.to_vec();

        handles.push(tokio::spawn(async move {
            let req = Request::builder()
                .method("PUT")
                .uri(&upload_path)
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .header(header::CONTENT_TYPE, "application/octet-stream")
                .body(Body::from(payload_vec))
                .unwrap();
            router_clone.oneshot(req).await.unwrap()
        }));
    }

    let results = futures_util::future::join_all(handles).await;
    for res in results {
        let resp = res.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "All concurrent duplicate uploads must return 200 OK"
        );
    }

    // Invariant: Exactly one mapping exists in DB for this repo and OID
    let lfs_obj = db
        .get_lfs_object_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap()
        .expect("Object mapping must exist");
    assert_eq!(lfs_obj.oid, oid);
    assert_eq!(lfs_obj.size_bytes, payload.len() as i64);
}

#[tokio::test]
async fn test_lfs_upload_gc_reachability_and_preservation() {
    let (db, state, router, user, _pat_secret, repo) = setup_test_env().await;

    let payload = b"LFS_GC_REACHABILITY_ACTIVE_PAYLOAD";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    let (token, _) = generate_scoped_token(&repo.id, &oid, "upload", payload.len() as u64, 3600);
    let upload_path = format!(
        "/{}/{}.git/info/lfs/objects/{}",
        user.username, repo.name, oid
    );

    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 1. Committed LFS object has manifest ID
    let lfs_obj = db
        .get_lfs_object_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap()
        .expect("LFS object mapping exists");

    let manifest = state
        .artifacts
        .store()
        .get_manifest(&lfs_obj.manifest_id)
        .unwrap()
        .expect("Manifest exists in artifact store");
    assert!(!manifest.chunks.is_empty(), "Manifest must have chunks");

    let chunk_hashes: Vec<String> = manifest
        .chunks
        .iter()
        .map(|c| c.chunk_hash.clone())
        .collect();

    // 2. Add an unreferenced orphan chunk directly to the store (simulating an interrupted upload)
    let orphan_bytes = b"ORPHAN_UNCOMMITTED_CHUNK_SHOULD_BE_COLLECTED";
    let orphan_hash = blake3::hash(orphan_bytes).to_hex().to_string();
    state
        .artifacts
        .store()
        .store_chunk(&orphan_hash, orphan_bytes)
        .unwrap();

    let mut all_scanned_chunks = chunk_hashes.clone();
    all_scanned_chunks.push(orphan_hash.clone());

    // 3. Run GC tracing active manifests from DB
    let active_manifest_ids = vec![lfs_obj.manifest_id.clone()];
    let report = ArtifactGcCollector::collect_garbage(
        state.artifacts.store(),
        &active_manifest_ids,
        &all_scanned_chunks,
        true, // dry run
    )
    .unwrap();

    assert_eq!(
        report.reachable_chunks,
        chunk_hashes.len(),
        "All chunks belonging to active LFS object must be reachable"
    );
    assert_eq!(
        report.candidate_chunks, 1,
        "Orphaned chunk must be identified as candidate for GC"
    );
    assert_eq!(
        report.reclaimed_bytes,
        orphan_bytes.len() as u64,
        "Reclaimed bytes must equal orphan chunk size"
    );

    // Re-verify that reachable chunks are untouched and retrievable
    for ch in &chunk_hashes {
        assert!(
            state.artifacts.store().get_chunk(ch).unwrap().is_some(),
            "LFS chunk {} must remain intact in CAS",
            ch
        );
    }
}

#[tokio::test]
async fn test_lfs_upload_in_flight_protected_from_concurrent_gc() {
    let (_db, state, _router, _user, _pat_secret, _repo) = setup_test_env().await;

    // Simulate an in-flight upload: chunk is written to CAS, but manifest/DB mapping not yet committed
    let in_flight_bytes = b"IN_FLIGHT_STREAMING_UPLOAD_CHUNK_BYTES";
    let in_flight_hash = blake3::hash(in_flight_bytes).to_hex().to_string();
    state
        .artifacts
        .store()
        .store_chunk(&in_flight_hash, in_flight_bytes)
        .unwrap();

    let all_scanned_chunks = vec![in_flight_hash.clone()];
    let active_manifest_ids: Vec<String> = Vec::new(); // No active manifest yet!

    // 1. Run concurrent GC with standard grace period (e.g. 300s)
    let report_with_grace = ArtifactGcCollector::collect_garbage_with_grace(
        state.artifacts.store(),
        &active_manifest_ids,
        &all_scanned_chunks,
        300,  // 300 seconds grace period
        true, // dry run
    )
    .unwrap();

    assert_eq!(
        report_with_grace.grace_protected_chunks, 1,
        "In-flight recently modified chunk must be protected by grace period"
    );
    assert_eq!(
        report_with_grace.candidate_chunks, 0,
        "Protected in-flight chunk must NOT be a candidate for deletion"
    );
    assert_eq!(
        report_with_grace.reclaimed_bytes, 0,
        "Zero bytes must be reclaimed while chunk is in grace period"
    );
    assert!(
        state.artifacts.store().has_chunk(&in_flight_hash),
        "In-flight chunk must remain present in CAS store"
    );

    // 2. Without grace period (or once grace expires), orphaned unmanifested chunk becomes candidate
    let report_expired = ArtifactGcCollector::collect_garbage_with_grace(
        state.artifacts.store(),
        &active_manifest_ids,
        &all_scanned_chunks,
        0, // Zero grace period (simulating expired grace)
        true,
    )
    .unwrap();

    assert_eq!(
        report_expired.grace_protected_chunks, 0,
        "Expired chunk is no longer grace-protected"
    );
    assert_eq!(
        report_expired.candidate_chunks, 1,
        "Expired unmanifested chunk must become candidate for collection"
    );
    assert_eq!(
        report_expired.reclaimed_bytes,
        in_flight_bytes.len() as u64,
        "Reclaimed bytes must match orphaned chunk size"
    );
}

#[tokio::test]
async fn test_lfs_upload_crash_between_cas_and_manifest_orphans_cleaned_up() {
    let (db, state, router, user, _pat_secret, repo) = setup_test_env().await;

    let payload = b"CRASH_AFTER_CHUNK_WRITE_BEFORE_MANIFEST";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // Write chunk directly to CAS to simulate crash right after chunk write
    let chunk_hash = blake3::hash(payload).to_hex().to_string();
    state
        .artifacts
        .store()
        .store_chunk(&chunk_hash, payload)
        .unwrap();

    // 1. Verify object is NOT published
    let found = db.get_lfs_object_by_repo_oid(&repo.id, &oid).await.unwrap();
    assert!(
        found.is_none(),
        "Unfinished upload must never be published in lfs_objects"
    );

    let found_man = db
        .get_artifact_manifest_by_repo_oid(&repo.id, &oid)
        .await
        .unwrap();
    assert!(
        found_man.is_none(),
        "Unfinished upload must never resolve a manifest"
    );

    // 2. Client batch download returns 404
    let batch_req = serde_json::json!({
        "operation": "download",
        "transfers": ["basic"],
        "objects": [{ "oid": &oid, "size": payload.len() }]
    });
    let (token, _) = generate_scoped_token(&repo.id, &oid, "download", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", token))
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        batch_json["objects"][0]["error"]["code"], 404,
        "Unmanifested object must return 404 in batch download"
    );

    // 3. GC cleanly reclaims the orphaned chunk once grace expires
    let active_manifests: Vec<String> = Vec::new();
    let all_chunks = vec![chunk_hash.clone()];
    let report = ArtifactGcCollector::collect_garbage_with_grace(
        state.artifacts.store(),
        &active_manifests,
        &all_chunks,
        0, // expired grace
        true,
    )
    .unwrap();
    assert_eq!(report.candidate_chunks, 1);
    assert_eq!(report.reclaimed_bytes, payload.len() as u64);
}

#[tokio::test]
async fn test_lfs_upload_crash_between_manifest_and_lfs_objects_leaves_object_unpublished() {
    let (db, state, router, user, pat_secret, repo) = setup_test_env().await;

    let payload = b"CRASH_AFTER_MANIFEST_BEFORE_LFS_OBJECT_MAPPING";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // Ingest into CAS and save artifact manifest, but DO NOT create lfs_objects mapping (simulating crash)
    let stored = state
        .artifacts
        .store_object_reader(
            "user",
            &user.id,
            &oid,
            Some("application/octet-stream".to_string()),
            std::io::Cursor::new(payload),
            Some(payload.len() as u64),
        )
        .await
        .unwrap();
    assert_eq!(stored.artifact_id, oid);

    // Invariant: lfs_objects mapping does not exist yet
    let mapping = db.get_lfs_object_by_repo_oid(&repo.id, &oid).await.unwrap();
    assert!(
        mapping.is_none(),
        "Object mapping must not exist prior to lfs_objects insert"
    );

    // 1. Direct download returns 404
    let (dl_token, _) =
        generate_scoped_token(&repo.id, &oid, "download", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Direct download must return 404 when lfs_objects mapping is missing"
    );

    // 2. Batch upload offers upload action (no existence leak)
    let basic_auth = format!(
        "Basic {}",
        encode_base64(format!("{}:{}", user.username, pat_secret).as_bytes())
    );
    let batch_req = serde_json::json!({
        "operation": "upload",
        "transfers": ["basic"],
        "objects": [{ "oid": &oid, "size": payload.len() }]
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/batch",
            user.username, repo.name
        ))
        .header(header::AUTHORIZATION, &basic_auth)
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&batch_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    let batch_json: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();
    assert!(
        !batch_json["objects"][0]["actions"]["upload"].is_null(),
        "Batch upload must offer upload action for unmapped object"
    );

    // 3. Client retries upload -> creates lfs_objects mapping
    let upload_href = batch_json["objects"][0]["actions"]["upload"]["href"]
        .as_str()
        .unwrap();
    let upload_path = url::Url::parse(upload_href).unwrap().path().to_string();
    let upload_auth = batch_json["objects"][0]["actions"]["upload"]["header"]["Authorization"]
        .as_str()
        .unwrap();

    let req = Request::builder()
        .method("PUT")
        .uri(&upload_path)
        .header(header::AUTHORIZATION, upload_auth)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Now object mapping exists and download succeeds!
    let mapping_after = db.get_lfs_object_by_repo_oid(&repo.id, &oid).await.unwrap();
    assert!(
        mapping_after.is_some(),
        "Object mapping must exist after retry"
    );

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded[..], payload);
}

#[tokio::test]
async fn test_lfs_cross_repository_token_reuse_rejected_across_all_routes() {
    let (db, state, router, alice, _alice_pat, _default_repo) = setup_test_env().await;

    let repo_a = state
        .repositories
        .create_repository(&alice, "repo-token-a", None, true, None)
        .await
        .unwrap();

    let (bob, _bob_pat) = create_test_user_and_pat(
        &db,
        "usr_bob_tok",
        "bob_tok",
        "bob_tok@spine.build",
        "sp_pat_bob_tok_12345",
    )
    .await;

    let repo_b = state
        .repositories
        .create_repository(&bob, "repo-token-b", None, true, None)
        .await
        .unwrap();

    let payload = b"CROSS_REPOSITORY_TOKEN_REUSE_PAYLOAD";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // Generate tokens specifically scoped to repo_a
    let (upload_token_a, _) =
        generate_scoped_token(&repo_a.id, &oid, "upload", payload.len() as u64, 3600);
    let (download_token_a, _) =
        generate_scoped_token(&repo_a.id, &oid, "download", payload.len() as u64, 3600);
    let (verify_token_a, _) =
        generate_scoped_token(&repo_a.id, &oid, "verify", payload.len() as u64, 3600);

    // 1. Attempt upload on repo_b with repo_a's upload token -> 403 Forbidden
    let req = Request::builder()
        .method("PUT")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            bob.username, repo_b.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Upload with cross-repo token must be rejected with 403 Forbidden"
    );

    // 2. Attempt download on repo_b with repo_a's download token -> 403 Forbidden
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            bob.username, repo_b.name, oid
        ))
        .header(
            header::AUTHORIZATION,
            format!("Bearer {}", download_token_a),
        )
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Download with cross-repo token must be rejected with 403 Forbidden"
    );

    // 3. Attempt verify on repo_b with repo_a's verify token -> 403 Forbidden
    let verify_req = serde_json::json!({ "oid": &oid, "size": payload.len() });
    let req = Request::builder()
        .method("POST")
        .uri(format!(
            "/{}/{}.git/info/lfs/verify",
            bob.username, repo_b.name
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", verify_token_a))
        .header(header::CONTENT_TYPE, LFS_MEDIA_TYPE)
        .body(Body::from(serde_json::to_vec(&verify_req).unwrap()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Verify with cross-repo token must be rejected with 403 Forbidden"
    );
}

#[tokio::test]
async fn test_lfs_upload_in_flight_active_longer_than_grace_protected_from_repeated_gc() {
    let (_db, state, _router, _user, _pat_secret, _repo) = setup_test_env().await;

    // 1. Simulate an in-flight upload that writes a chunk to CAS and registers an explicit lease
    let chunk_bytes = b"LONG_RUNNING_IN_FLIGHT_CHUNK_EXCEEDING_GRACE_PERIOD";
    let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();
    state
        .artifacts
        .store()
        .store_chunk(&chunk_hash, chunk_bytes)
        .unwrap();

    let lease_id = "lease_long_running_upload";
    state
        .artifacts
        .in_flight_registry()
        .register_lease(lease_id, 300); // 300s TTL
    state
        .artifacts
        .in_flight_registry()
        .pin_chunk(lease_id, &chunk_hash);

    let all_scanned_chunks = vec![chunk_hash.clone()];
    let active_manifest_ids: Vec<String> = Vec::new(); // No committed manifest yet!

    // 2. Run GC repeatedly with grace_secs = 0 (simulating upload duration > grace interval)
    // The explicit lease MUST protect the chunk on every GC pass
    for i in 0..4 {
        let report = ArtifactGcCollector::collect_garbage_with_registry(
            state.artifacts.store(),
            &active_manifest_ids,
            &all_scanned_chunks,
            Some(state.artifacts.in_flight_registry()),
            0, // 0s grace period: mtime grace alone would NOT protect this!
            false,
        )
        .unwrap();

        assert_eq!(
            report.in_flight_protected_chunks, 1,
            "Pass {}: chunk must be protected by explicit in-flight lease",
            i
        );
        assert_eq!(report.candidate_chunks, 0);
        assert_eq!(report.reclaimed_bytes, 0);
        assert!(
            state.artifacts.store().has_chunk(&chunk_hash),
            "Pass {}: chunk must remain in CAS",
            i
        );
    }

    // 3. Test cancellation / abort: if client aborts, lease is cancelled
    state.artifacts.in_flight_registry().cancel_lease(lease_id);

    // Now GC runs: without the lease and with 0 grace, the uncommitted chunk is reclaimed
    let report_after_cancel = ArtifactGcCollector::collect_garbage_with_registry(
        state.artifacts.store(),
        &active_manifest_ids,
        &all_scanned_chunks,
        Some(state.artifacts.in_flight_registry()),
        0,
        false,
    )
    .unwrap();
    assert_eq!(report_after_cancel.in_flight_protected_chunks, 0);
    assert_eq!(report_after_cancel.candidate_chunks, 1);
    assert_eq!(
        report_after_cancel.reclaimed_bytes,
        chunk_bytes.len() as u64
    );

    // 4. Test crash expiry: a lease that was not renewed expires
    let crash_lease_id = "lease_crashed_worker";
    state
        .artifacts
        .in_flight_registry()
        .register_lease(crash_lease_id, 1); // 1s TTL

    let crash_bytes = b"CRASHED_WORKER_CHUNK";
    let crash_hash = blake3::hash(crash_bytes).to_hex().to_string();
    state
        .artifacts
        .store()
        .store_chunk(&crash_hash, crash_bytes)
        .unwrap();
    state
        .artifacts
        .in_flight_registry()
        .pin_chunk(crash_lease_id, &crash_hash);

    // Wait for 1s TTL to expire
    tokio::time::sleep(tokio::time::Duration::from_millis(1100)).await;

    let all_crash_chunks = vec![crash_hash.clone()];
    let report_crash = ArtifactGcCollector::collect_garbage_with_registry(
        state.artifacts.store(),
        &active_manifest_ids,
        &all_crash_chunks,
        Some(state.artifacts.in_flight_registry()),
        0,
        false,
    )
    .unwrap();
    assert_eq!(report_crash.in_flight_protected_chunks, 0);
    assert_eq!(report_crash.candidate_chunks, 1);
    assert_eq!(report_crash.reclaimed_bytes, crash_bytes.len() as u64);
}

#[tokio::test]
async fn test_publication_visibility_only_after_cas_and_repo_mapping_durable() {
    let (db, state, router, user, _pat_secret, repo) = setup_test_env().await;

    let payload = b"DURABLE_VISIBILITY_INVARIANT_PAYLOAD";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    let (dl_token, _) =
        generate_scoped_token(&repo.id, &oid, "download", payload.len() as u64, 3600);

    // Phase 1: Zero data exists -> 404
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Phase 2: Chunks and Manifest exist in CAS, but NOT mapped in lfs_objects
    let stored = state
        .artifacts
        .store_object_reader(
            "user",
            &user.id,
            &oid,
            Some("application/octet-stream".to_string()),
            std::io::Cursor::new(payload.to_vec()),
            Some(payload.len() as u64),
        )
        .await
        .unwrap();

    // Verify mapping is not present
    let mapping = db.get_lfs_object_by_repo_oid(&repo.id, &oid).await.unwrap();
    assert!(mapping.is_none(), "Mapping must not exist yet");

    // Visibility test: Object MUST NOT be visible via batch or direct GET
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Object without repo mapping must be invisible (404)"
    );

    // Phase 3: Now persist repository mapping in lfs_objects
    let lfs_obj = LfsObject {
        id: format!("lfs_{}", uuid::Uuid::new_v4().simple()),
        repository_id: repo.id.clone(),
        oid: oid.clone(),
        size_bytes: payload.len() as i64,
        manifest_id: stored.id.clone(),
        created_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    };
    db.save_lfs_object(lfs_obj).await.unwrap();

    // Visibility test: Now BOTH CAS and mapping are durable -> object is VISIBLE and downloadable!
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            user.username, repo.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded[..], payload);
}

#[tokio::test]
async fn test_multitenant_shared_cas_deduplication_and_gc_reachability() {
    let (db, state, router, alice, _alice_pat, _default_repo) = setup_test_env().await;

    let repo_a = state
        .repositories
        .create_repository(&alice, "repo-shared-a", None, true, None)
        .await
        .unwrap();

    let (bob, _bob_pat) = create_test_user_and_pat(
        &db,
        "usr_bob_shared",
        "bob_shared",
        "bob_shared@spine.build",
        "test_pat_bob_shared",
    )
    .await;

    let repo_b = state
        .repositories
        .create_repository(&bob, "repo-shared-b", None, true, None)
        .await
        .unwrap();

    let (charlie, _charlie_pat) = create_test_user_and_pat(
        &db,
        "usr_charlie_shared",
        "charlie_shared",
        "charlie_shared@spine.build",
        "test_pat_charlie_shared",
    )
    .await;

    let repo_c = state
        .repositories
        .create_repository(&charlie, "repo-shared-c", None, true, None)
        .await
        .unwrap();

    let payload = b"DEDUPLICATED_SHARED_CAS_MULTI_TENANT_OBJECT_BYTES";
    let mut hasher = Sha256::new();
    hasher.update(payload);
    let oid = hex::encode(hasher.finalize());

    // 1. Repo A ingests the shared object
    let (upload_token_a, _) =
        generate_scoped_token(&repo_a.id, &oid, "upload", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("PUT")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            alice.username, repo_a.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_a))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // 2. Repo B ingests the identical shared object -> deduplication!
    let (upload_token_b, _) =
        generate_scoped_token(&repo_b.id, &oid, "upload", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("PUT")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            bob.username, repo_b.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", upload_token_b))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(payload.to_vec()))
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Verify both Repo A and Repo B resolve the object and point to the same manifest
    let obj_a = db
        .get_lfs_object_by_repo_oid(&repo_a.id, &oid)
        .await
        .unwrap()
        .expect("Repo A object");
    let obj_b = db
        .get_lfs_object_by_repo_oid(&repo_b.id, &oid)
        .await
        .unwrap()
        .expect("Repo B object");
    assert_eq!(
        obj_a.manifest_id, obj_b.manifest_id,
        "Both repos must point to the identical shared deduplicated manifest"
    );

    // Repo C (unauthorized) cannot download or resolve object
    let (dl_token_c, _) =
        generate_scoped_token(&repo_c.id, &oid, "download", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            charlie.username, repo_c.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token_c))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Repo C must not resolve object belonging only to Repo A and B"
    );

    // 3. GC Reachability Check with shared references
    let shared_manifest = state
        .artifacts
        .store()
        .get_manifest(&obj_a.manifest_id)
        .unwrap()
        .unwrap();
    let shared_chunk_hashes: Vec<String> = shared_manifest
        .chunks
        .iter()
        .map(|c| c.chunk_hash.clone())
        .collect();

    // With both repos referencing the manifest, GC reachability retains chunks
    let rep1 = ArtifactGcCollector::collect_garbage(
        state.artifacts.store(),
        std::slice::from_ref(&obj_a.manifest_id),
        &shared_chunk_hashes,
        false,
    )
    .unwrap();
    assert_eq!(rep1.reachable_chunks, shared_chunk_hashes.len());
    assert_eq!(rep1.candidate_chunks, 0);

    // 4. Delete Repo A's mapping:
    let deleted_a = db.delete_lfs_object(&repo_a.id, &oid).await.unwrap();
    assert!(deleted_a, "Repo A LFS mapping must be deleted");

    // Invariant 1: Repo A gets 404 Not Found on download
    let (dl_token_a_post, _) =
        generate_scoped_token(&repo_a.id, &oid, "download", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            alice.username, repo_a.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token_a_post))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::NOT_FOUND,
        "Repo A must return 404 after mapping is deleted"
    );

    // Invariant 2: Repo B still references the manifest and CAN download the bytes!
    let (dl_token_b, _) =
        generate_scoped_token(&repo_b.id, &oid, "download", payload.len() as u64, 3600);
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "/{}/{}.git/info/lfs/objects/{}",
            bob.username, repo_b.name, oid
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", dl_token_b))
        .body(Body::empty())
        .unwrap();
    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let downloaded_b = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    assert_eq!(&downloaded_b[..], payload);

    // Invariant 3: GC check - Repo B's manifest preserves shared chunks!
    let rep2 = ArtifactGcCollector::collect_garbage(
        state.artifacts.store(),
        std::slice::from_ref(&obj_b.manifest_id),
        &shared_chunk_hashes,
        false,
    )
    .unwrap();
    assert_eq!(rep2.reachable_chunks, shared_chunk_hashes.len());
    assert_eq!(rep2.candidate_chunks, 0);

    // Invariant 4: Now delete Repo B's mapping as well
    let deleted_b = db.delete_lfs_object(&repo_b.id, &oid).await.unwrap();
    assert!(deleted_b, "Repo B LFS mapping must be deleted");

    // Both mappings deleted: GC with expired grace now reclaims orphan CAS chunks!
    let rep3 = ArtifactGcCollector::collect_garbage_with_grace(
        state.artifacts.store(),
        &[], // no active manifests
        &shared_chunk_hashes,
        0, // expired grace
        false,
    )
    .unwrap();
    assert_eq!(rep3.reachable_chunks, 0);
    assert_eq!(rep3.candidate_chunks, shared_chunk_hashes.len());
    assert_eq!(rep3.reclaimed_bytes, payload.len() as u64);
}
