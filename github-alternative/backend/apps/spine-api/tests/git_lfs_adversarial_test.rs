use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use sha2::{Digest, Sha256};
use spine_api::api::routes_lfs::{generate_scoped_token, LFS_MEDIA_TYPE};
use spine_api::api::{build_router, AppState};
use spine_api::auth::pat::hash_pat;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
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
        .get_artifact_manifest_by_oid(&legitimate_oid)
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
    let found = db.get_artifact_manifest_by_oid(&oid).await.unwrap();
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
        .get_artifact_manifest_by_oid(&oid)
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
        .get_artifact_manifest_by_oid(&oid)
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
