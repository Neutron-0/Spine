use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use sha2::Digest;
use spine_api::api::streaming::{format_content_disposition, stream_body_to_tempfile};
use spine_api::api::{build_router, AppState};
use spine_api::core::error::ForgeError;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_streaming_body_to_tempfile_success_and_digest() {
    let raw_data = b"STREAMING_PAYLOAD_CHUNK_1_CHUNK_2_COMPUTED_CORRECTLY";
    let body = Body::from(raw_data.to_vec());
    let (mut tempfile, total_bytes, sha256_hex) =
        stream_body_to_tempfile(body, 1024, Duration::from_secs(10))
            .await
            .expect("Streaming valid body should succeed");

    assert_eq!(total_bytes, raw_data.len() as u64);
    let expected_sha256 = hex::encode(sha2::Sha256::digest(raw_data));
    assert_eq!(sha256_hex, expected_sha256);

    use std::io::Read;
    let mut file_content = Vec::new();
    tempfile.read_to_end(&mut file_content).unwrap();
    assert_eq!(file_content, raw_data);
}

#[tokio::test]
async fn test_streaming_body_rejects_oversized_payload() {
    let raw_data = vec![0x41u8; 500]; // 500 bytes
    let body = Body::from(raw_data);
    // Limit to 200 bytes
    let result = stream_body_to_tempfile(body, 200, Duration::from_secs(10)).await;
    match result {
        Err(ForgeError::BadRequest(msg)) => {
            assert!(
                msg.contains("exceeds maximum allowed size"),
                "Expected size limit error, got: {}",
                msg
            );
        }
        other => panic!("Expected ForgeError::BadRequest, got {:?}", other),
    }
}

#[test]
fn test_format_content_disposition_sanitization_and_unicode() {
    // 1. Clean ASCII filename
    let cd1 = format_content_disposition("attachment", "release-v1.0.tar.gz");
    assert_eq!(cd1, "attachment; filename=\"release-v1.0.tar.gz\"");

    // 2. Filename with quotes, CRLF, and traversal characters
    let malicious = "evil\r\n\"Injected: true\"/..\\payload.exe";
    let cd2 = format_content_disposition("attachment", malicious);
    assert!(!cd2.contains('\r'));
    assert!(!cd2.contains('\n'));
    assert_eq!(
        cd2,
        "attachment; filename=\"evil___Injected: true__.._payload.exe\""
    );

    // 3. Unicode Cyrillic filename: must produce ASCII fallback and RFC 5987 filename*
    let unicode_name = "релиз-v1.0.zip";
    let cd3 = format_content_disposition("attachment", unicode_name);
    assert!(cd3.starts_with("attachment; filename=\""));
    assert!(cd3.contains("; filename*=UTF-8''"));
    // Header value must be purely valid ASCII
    assert!(
        cd3.is_ascii(),
        "Content-Disposition header must be pure ASCII"
    );
}

#[tokio::test]
async fn test_upload_concurrency_limiting_via_semaphore() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();
    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db, &storage_root, None).unwrap();

    let mut permits = Vec::new();
    // AppState semaphore has 16 permits
    for _ in 0..16 {
        permits.push(
            state
                .acquire_upload_permit()
                .await
                .expect("Permit should be available"),
        );
    }

    // 17th request must be rejected with RateLimited (HTTP 429)
    let rejected = state.acquire_upload_permit().await;
    match rejected {
        Err(ForgeError::RateLimited(msg)) => {
            assert!(msg.contains("concurrency limit"), "Got: {}", msg);
        }
        other => panic!("Expected RateLimited error, got {:?}", other),
    }

    // Drop one permit
    permits.pop();

    // Now permit acquisition must succeed again
    let acquired = state.acquire_upload_permit().await;
    assert!(
        acquired.is_ok(),
        "Permit acquisition should succeed after release"
    );
}

#[tokio::test]
async fn test_upfront_auth_on_large_upload_routes() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();
    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();
    let router = build_router(state);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // Create Alice (repo owner)
    let alice = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice Owner".to_string(),
        password_hash: "hash_alice".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(alice.clone()).await.unwrap();

    // Create Bob (attacker)
    let bob = User {
        id: "usr_bob".to_string(),
        username: "bob".to_string(),
        normalized_username: "bob".to_string(),
        email: "bob@spine.build".to_string(),
        normalized_email: "bob@spine.build".to_string(),
        display_name: "Bob Attacker".to_string(),
        password_hash: "hash_bob".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(bob.clone()).await.unwrap();

    let pat_alice_plain = "sp_pat_alice_stream_test_1234567890abcdef";
    let pat_alice = PersonalAccessToken {
        id: "pat_alice_stream".to_string(),
        user_id: alice.id.clone(),
        name: "alice_stream".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_alice_plain),
        scopes: vec!["repo".to_string(), "admin".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_alice).await.unwrap();

    let pat_bob_plain = "sp_pat_bob_stream_test_1234567890abcdef";
    let pat_bob = PersonalAccessToken {
        id: "pat_bob_stream".to_string(),
        user_id: bob.id.clone(),
        name: "bob_stream".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_bob_plain),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_bob).await.unwrap();

    // Create private repository for Alice
    let repo_alice = ForgeRepository {
        id: "repo_alice_secret".to_string(),
        owner_user_id: alice.id.clone(),
        owner_username: alice.username.clone(),
        owner_org_id: None,
        name: "secret-code".to_string(),
        normalized_name: "secret-code".to_string(),
        description: "Confidential".to_string(),
        is_private: true,
        default_branch: "main".to_string(),
        native_repo_id: "alice_secret_native".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };
    db.create_repository(repo_alice.clone()).await.unwrap();

    // Create a release for Alice's repo
    let release_alice = Release {
        id: "rel_alice_v1".to_string(),
        repo_id: repo_alice.id.clone(),
        tag_name: "v1.0.0".to_string(),
        target_commit_id: "sha_1111111111111111111111111111111111111111".to_string(),
        title: "v1.0.0 Release".to_string(),
        body: "Notes".to_string(),
        is_draft: false,
        is_prerelease: false,
        author_id: alice.id.clone(),
        created_at_nanos: now,
        updated_at_nanos: now,
        published_at_nanos: Some(now),
    };
    db.create_release(release_alice).await.unwrap();

    // 1. Release asset upload: Anonymous -> 401 Unauthorized
    let anon_upload_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-code/releases/rel_alice_v1/assets?name=test.bin")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"DATA".to_vec()))
        .unwrap();
    let anon_upload_res = router.clone().oneshot(anon_upload_req).await.unwrap();
    assert_eq!(
        anon_upload_res.status(),
        StatusCode::UNAUTHORIZED,
        "Anonymous release asset upload must return 401"
    );

    // 2. Release asset upload: Bob (unauthorized) -> 404 Not Found (private repo obfuscation)
    let bob_upload_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-code/releases/rel_alice_v1/assets?name=test.bin")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"ATTACK_PAYLOAD".to_vec()))
        .unwrap();
    let bob_upload_res = router.clone().oneshot(bob_upload_req).await.unwrap();
    assert_eq!(
        bob_upload_res.status(),
        StatusCode::NOT_FOUND,
        "Unauthorized release asset upload must return 404"
    );

    // 3. Git Receive-Pack: Anonymous -> 401 Unauthorized
    let anon_git_req = Request::builder()
        .method("POST")
        .uri("/alice/secret-code.git/git-receive-pack")
        .header(
            header::CONTENT_TYPE,
            "application/x-git-receive-pack-request",
        )
        .body(Body::from(b"0000".to_vec()))
        .unwrap();
    let anon_git_res = router.clone().oneshot(anon_git_req).await.unwrap();
    assert_eq!(
        anon_git_res.status(),
        StatusCode::UNAUTHORIZED,
        "Anonymous git-receive-pack must return 401"
    );

    // 4. Git Receive-Pack: Bob -> 403 Forbidden or 404 Not Found
    let bob_git_req = Request::builder()
        .method("POST")
        .uri("/alice/secret-code.git/git-receive-pack")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .header(
            header::CONTENT_TYPE,
            "application/x-git-receive-pack-request",
        )
        .body(Body::from(b"0000".to_vec()))
        .unwrap();
    let bob_git_res = router.clone().oneshot(bob_git_req).await.unwrap();
    assert!(
        bob_git_res.status() == StatusCode::FORBIDDEN
            || bob_git_res.status() == StatusCode::NOT_FOUND,
        "Unauthorized git-receive-pack must return 403 or 404; got {}",
        bob_git_res.status()
    );

    // 5. Git LFS Object Upload: Anonymous -> 401 Unauthorized
    let anon_lfs_req = Request::builder()
        .method("PUT")
        .uri("/alice/secret-code.git/info/lfs/objects/0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"LFS_PAYLOAD".to_vec()))
        .unwrap();
    let anon_lfs_res = router.clone().oneshot(anon_lfs_req).await.unwrap();
    assert_eq!(
        anon_lfs_res.status(),
        StatusCode::UNAUTHORIZED,
        "Anonymous git-lfs upload must return 401"
    );

    // 6. Artifact Namespace Upload: Create private namespace for Alice, Bob attempts upload -> 403 Forbidden
    let ns_alice = ArtifactNamespace {
        id: "ns_alice_id".to_string(),
        name: "alice".to_string(),
        description: "Alice namespace".to_string(),
        owner_type: "USER".to_string(),
        owner_id: alice.id.clone(),
        is_private: true,
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.create_artifact_namespace(ns_alice).await.unwrap();

    let bob_art_req = Request::builder()
        .method("POST")
        .uri("/api/v1/artifacts/alice/my-lib/versions/1.0.0")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"ARTIFACT_BYTES".to_vec()))
        .unwrap();
    let bob_art_res = router.clone().oneshot(bob_art_req).await.unwrap();
    assert_eq!(
        bob_art_res.status(),
        StatusCode::FORBIDDEN,
        "Bob uploading to Alice's namespace must return 403 Forbidden"
    );

    // 7. Legitimate Upload with Unicode Filename: Alice uploads asset with Unicode name
    let encoded_name = "release-%D0%B8%D0%BD%D0%B6%D0%B5%D0%BD%D0%B5%D1%80-v1.0.tar.gz";
    let alice_upload_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/secret-code/releases/rel_alice_v1/assets?name={}",
            encoded_name
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/gzip")
        .body(Body::from(b"LEGITIMATE_RELEASE_PAYLOAD_GZIP".to_vec()))
        .unwrap();
    let alice_upload_res = router.clone().oneshot(alice_upload_req).await.unwrap();
    assert_eq!(
        alice_upload_res.status(),
        StatusCode::CREATED,
        "Legitimate release asset upload with Unicode filename should return 201 Created"
    );

    let res_body = to_bytes(alice_upload_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let asset_json: serde_json::Value = serde_json::from_slice(&res_body).unwrap();
    let asset_id = asset_json["id"].as_str().unwrap();

    // 8. Download asset and verify Content-Disposition header format
    let alice_dl_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/repos/alice/secret-code/releases/rel_alice_v1/assets/{}/download",
            asset_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .body(Body::empty())
        .unwrap();
    let alice_dl_res = router.clone().oneshot(alice_dl_req).await.unwrap();
    assert_eq!(alice_dl_res.status(), StatusCode::OK);

    let cd_header = alice_dl_res
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .expect("Content-Disposition header must be present")
        .to_str()
        .expect("Content-Disposition must be valid ASCII string");

    assert!(cd_header.starts_with("attachment; filename=\""));
    assert!(cd_header.contains("filename*=UTF-8''"));

    // 9. Bob attempts download of Alice's private release asset -> 404 Not Found
    let bob_dl_req = Request::builder()
        .method("GET")
        .uri(format!(
            "/api/v1/repos/alice/secret-code/releases/rel_alice_v1/assets/{}/download",
            asset_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_dl_res = router.clone().oneshot(bob_dl_req).await.unwrap();
    assert_eq!(
        bob_dl_res.status(),
        StatusCode::NOT_FOUND,
        "Bob must not download Alice's private release asset"
    );
}
