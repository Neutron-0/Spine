use axum::body::{to_bytes, Body};
use axum::http::{header, Request, StatusCode};
use serde_json::json;
use spine_api::api::{build_router, AppState};
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_security_authorization_and_cross_tenant_isolation_regression() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();
    let db = Arc::new(InMemoryDatabase::new());

    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();
    let router = build_router(state);

    // 1. Create two users: Alice (legitimate owner) and Bob (attacker / non-member)
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
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    };
    db.create_user(alice.clone()).await.unwrap();

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
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    };
    db.create_user(bob.clone()).await.unwrap();

    // Create session / PAT tokens for Alice and Bob
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let pat_alice_plain = "sp_pat_alice_1234567890abcdef1234567890abcdef";
    let pat_alice = PersonalAccessToken {
        id: "pat_alice_id".to_string(),
        user_id: alice.id.clone(),
        name: "alice_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_alice_plain),
        scopes: vec!["repo".to_string(), "admin".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_alice).await.unwrap();

    let pat_bob_plain = "sp_pat_bob_1234567890abcdef1234567890abcdef";
    let pat_bob = PersonalAccessToken {
        id: "pat_bob_id".to_string(),
        user_id: bob.id.clone(),
        name: "bob_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_bob_plain),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_bob).await.unwrap();

    // 2. Create private repository for Alice, and private repository for Bob
    let repo_alice = ForgeRepository {
        id: "repo_alice_secret".to_string(),
        owner_user_id: alice.id.clone(),
        owner_username: alice.username.clone(),
        owner_org_id: None,
        name: "secret-project".to_string(),
        normalized_name: "secret-project".to_string(),
        description: "Alice private proprietary source".to_string(),
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

    let repo_bob = ForgeRepository {
        id: "repo_bob_project".to_string(),
        owner_user_id: bob.id.clone(),
        owner_username: bob.username.clone(),
        owner_org_id: None,
        name: "bobs-project".to_string(),
        normalized_name: "bobs-project".to_string(),
        description: "Bob project".to_string(),
        is_private: true,
        default_branch: "main".to_string(),
        native_repo_id: "bob_project_native".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };
    db.create_repository(repo_bob.clone()).await.unwrap();

    // Create a release and an asset in Alice's private repository
    let create_rel_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-project/releases")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "tag_name": "v1.0.0",
                "target_commit_id": "sha_1111111111111111111111111111111111111111",
                "title": "Secret Release v1.0.0",
                "body": "Proprietary internal details"
            })
            .to_string(),
        ))
        .unwrap();

    let create_rel_res = router.clone().oneshot(create_rel_req).await.unwrap();
    assert_eq!(create_rel_res.status(), StatusCode::CREATED);
    let rel_body = to_bytes(create_rel_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let rel_json: serde_json::Value = serde_json::from_slice(&rel_body).unwrap();
    let release_id = rel_json["id"].as_str().unwrap().to_string();

    // Upload asset to Alice's release
    let upload_asset_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/secret-project/releases/{}/assets?name=payload.bin",
            release_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"PROPRIETARY_BINARY_PAYLOAD".to_vec()))
        .unwrap();

    let upload_asset_res = router.clone().oneshot(upload_asset_req).await.unwrap();
    assert_eq!(upload_asset_res.status(), StatusCode::CREATED);
    let asset_body = to_bytes(upload_asset_res.into_body(), usize::MAX)
        .await
        .unwrap();
    let asset_json: serde_json::Value = serde_json::from_slice(&asset_body).unwrap();
    let asset_id = asset_json["id"].as_str().unwrap().to_string();

    // -------------------------------------------------------------------------
    // TEST 1: Release & Asset Authorization on Private Repositories (FINDING-02)
    // -------------------------------------------------------------------------
    // 1a. Anonymous request to get release info must return 404 (existence obfuscated)
    let anon_get_rel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}",
                    release_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anon_get_rel.status(), StatusCode::NOT_FOUND);

    // 1b. Attacker Bob's request to get Alice's private release info must return 404
    let bob_get_rel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}",
                    release_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bob_get_rel.status(), StatusCode::NOT_FOUND);

    // 1c. Alice's authenticated request to get release info must return 200 OK
    let alice_get_rel = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}",
                    release_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alice_get_rel.status(), StatusCode::OK);

    // 1d. Anonymous download of private release asset must return 404
    let anon_download_asset = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}/assets/{}/download",
                    release_id, asset_id
                ))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(anon_download_asset.status(), StatusCode::NOT_FOUND);

    // 1e. Attacker Bob's download of private release asset must return 404
    let bob_download_asset = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}/assets/{}/download",
                    release_id, asset_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(bob_download_asset.status(), StatusCode::NOT_FOUND);

    // 1f. Alice's authenticated download of private release asset must return 200 OK
    let alice_download_asset = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/alice/secret-project/releases/{}/assets/{}/download",
                    release_id, asset_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(alice_download_asset.status(), StatusCode::OK);
    let downloaded_bytes = to_bytes(alice_download_asset.into_body(), usize::MAX)
        .await
        .unwrap();
    assert_eq!(downloaded_bytes, b"PROPRIETARY_BINARY_PAYLOAD".to_vec());

    // 1g. IDOR: Bob tries to download Alice's asset using Bob's repo path -> 404
    let idor_download = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "/api/v1/repos/bob/bobs-project/releases/{}/assets/{}/download",
                    release_id, asset_id
                ))
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(idor_download.status(), StatusCode::NOT_FOUND);

    // -------------------------------------------------------------------------
    // TEST 2: Cross-Tenant Deletion IDOR Protection (FINDING-03)
    // -------------------------------------------------------------------------
    // Create Branch Protection Rule in Alice's repository
    let bpr = BranchProtectionRule {
        id: "bpr_alice_rule".to_string(),
        repo_id: repo_alice.id.clone(),
        branch_pattern: "main".to_string(),
        require_pull_request: true,
        required_approvals: 2,
        prevent_self_approval: true,
        require_changes_addressed: true,
        dismiss_stale_reviews_on_push: true,
        restrict_merges_to_admins: false,
        required_status_checks: vec![],
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.set_branch_protection(bpr).await.unwrap();

    // Bob is admin of his own repo, but tries to delete Alice's branch protection rule
    let bob_del_bpr = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/repos/bob/bobs-project/protection/bpr_alice_rule")
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bob_del_bpr.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant branch protection deletion must return 404"
    );
    // Verify rule was not deleted
    let alice_rules = db
        .list_branch_protection_rules(&repo_alice.id)
        .await
        .unwrap();
    assert_eq!(alice_rules.len(), 1);

    // Create Label in Alice's repository
    let label = RepositoryLabel {
        id: "lbl_alice_critical".to_string(),
        repo_id: repo_alice.id.clone(),
        name: "critical".to_string(),
        color: "#ff0000".to_string(),
        description: "Critical bug".to_string(),
        created_at_nanos: now,
    };
    db.create_label(label).await.unwrap();

    // Bob tries to delete Alice's label via Bob's repo path
    let bob_del_label = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/repos/bob/bobs-project/labels/lbl_alice_critical")
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bob_del_label.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant label deletion must return 404"
    );
    // Verify label was not deleted
    let alice_labels = db.list_labels_for_repo(&repo_alice.id).await.unwrap();
    assert_eq!(alice_labels.len(), 1);

    // Create Milestone in Alice's repository
    let ms = Milestone {
        id: "ms_alice_v1".to_string(),
        repo_id: repo_alice.id.clone(),
        title: "Sprint 1".to_string(),
        description: "Sprint 1 milestone".to_string(),
        state: "OPEN".to_string(),
        due_date_nanos: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        closed_at_nanos: None,
    };
    db.create_milestone(ms).await.unwrap();

    // Bob tries to update Alice's milestone via Bob's repo path
    let bob_update_ms = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PATCH")
                .uri("/api/v1/repos/bob/bobs-project/milestones/ms_alice_v1")
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(json!({"title": "Hijacked Milestone"}).to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bob_update_ms.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant milestone update must return 404"
    );

    // Bob tries to delete Alice's milestone via Bob's repo path
    let bob_del_ms = router
        .clone()
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/api/v1/repos/bob/bobs-project/milestones/ms_alice_v1")
                .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        bob_del_ms.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant milestone deletion must return 404"
    );

    // -------------------------------------------------------------------------
    // TEST 3: Cargo & Package Registry Authorization (FINDING-01)
    // -------------------------------------------------------------------------
    // Build cargo package payload for alice
    let cargo_meta_alice = json!({
        "name": "alice-crypto",
        "vers": "0.1.0",
        "deps": [],
        "features": {},
        "authors": ["Alice"],
        "description": "Alice Crypto Library",
        "license": "MIT"
    });
    let meta_alice_bytes = serde_json::to_vec(&cargo_meta_alice).unwrap();
    let tarball_bytes = b"MOCK_CARGO_TARBALL_BYTES_FOR_ALICE";
    let mut payload_alice = Vec::new();
    payload_alice.extend_from_slice(&(meta_alice_bytes.len() as u32).to_le_bytes());
    payload_alice.extend_from_slice(&meta_alice_bytes);
    payload_alice.extend_from_slice(&(tarball_bytes.len() as u32).to_le_bytes());
    payload_alice.extend_from_slice(tarball_bytes);

    // Alice publishes "alice-crypto" v0.1.0
    let alice_pub_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/crates/new")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .body(Body::from(payload_alice))
        .unwrap();
    let alice_pub_res = router.clone().oneshot(alice_pub_req).await.unwrap();
    assert_eq!(alice_pub_res.status(), StatusCode::OK);

    // Bob tries to publish new version v0.2.0 to Alice's package "alice-crypto" -> 403 Forbidden
    let cargo_meta_bob_hijack = json!({
        "name": "alice-crypto",
        "vers": "0.2.0",
        "deps": [],
        "features": {},
        "authors": ["Bob Attacker"],
        "description": "Hijacked package version",
        "license": "MIT"
    });
    let meta_bob_bytes = serde_json::to_vec(&cargo_meta_bob_hijack).unwrap();
    let mut payload_bob = Vec::new();
    payload_bob.extend_from_slice(&(meta_bob_bytes.len() as u32).to_le_bytes());
    payload_bob.extend_from_slice(&meta_bob_bytes);
    payload_bob.extend_from_slice(&(tarball_bytes.len() as u32).to_le_bytes());
    payload_bob.extend_from_slice(tarball_bytes);

    let bob_pub_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/crates/new")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::from(payload_bob))
        .unwrap();
    let bob_pub_res = router.clone().oneshot(bob_pub_req).await.unwrap();
    assert_eq!(
        bob_pub_res.status(),
        StatusCode::FORBIDDEN,
        "Publishing new version to another user's crate must return 403 Forbidden"
    );

    // Bob tries to yank Alice's crate v0.1.0 -> 403 Forbidden
    let bob_yank_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/crates/alice-crypto/0.1.0/yank")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_yank_res = router.clone().oneshot(bob_yank_req).await.unwrap();
    assert_eq!(
        bob_yank_res.status(),
        StatusCode::FORBIDDEN,
        "Non-owner yanking another user's crate must return 403 Forbidden"
    );

    // Alice yanks her own crate v0.1.0 -> 200 OK
    let alice_yank_req = Request::builder()
        .method("DELETE")
        .uri("/api/v1/crates/alice-crypto/0.1.0/yank")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .body(Body::empty())
        .unwrap();
    let alice_yank_res = router.clone().oneshot(alice_yank_req).await.unwrap();
    assert_eq!(alice_yank_res.status(), StatusCode::OK);

    // Bob tries to unyank Alice's crate v0.1.0 -> 403 Forbidden
    let bob_unyank_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/crates/alice-crypto/0.1.0/unyank")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_unyank_res = router.clone().oneshot(bob_unyank_req).await.unwrap();
    assert_eq!(
        bob_unyank_res.status(),
        StatusCode::FORBIDDEN,
        "Non-owner unyanking another user's crate must return 403 Forbidden"
    );

    // Alice unyanks her own crate v0.1.0 -> 200 OK
    let alice_unyank_req = Request::builder()
        .method("PUT")
        .uri("/api/v1/crates/alice-crypto/0.1.0/unyank")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .body(Body::empty())
        .unwrap();
    let alice_unyank_res = router.clone().oneshot(alice_unyank_req).await.unwrap();
    assert_eq!(alice_unyank_res.status(), StatusCode::OK);
}
