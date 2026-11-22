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
                .body(Body::from(
                    json!({"title": "Hijacked Milestone"}).to_string(),
                ))
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

    // -------------------------------------------------------------------------
    // 7. Route Authorization: Webhooks Cross-Tenant Isolation
    // -------------------------------------------------------------------------
    // Alice creates a webhook for her private repository
    let create_hook_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-project/webhooks")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            json!({
                "target_url": "https://example.com/webhook",
                "secret": "hook-secret",
                "events": ["push"],
                "is_active": true
            })
            .to_string(),
        ))
        .unwrap();
    let create_hook_res = router.clone().oneshot(create_hook_req).await.unwrap();
    assert_eq!(create_hook_res.status(), StatusCode::CREATED);
    let hook_body: serde_json::Value = serde_json::from_slice(
        &to_bytes(create_hook_res.into_body(), usize::MAX)
            .await
            .unwrap(),
    )
    .unwrap();
    let webhook_id = hook_body["id"].as_str().unwrap();

    // Bob attempts to read Alice's webhook -> 404 / 403 Forbidden
    let bob_get_hook_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/webhooks/{}", webhook_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_get_hook_res = router.clone().oneshot(bob_get_hook_req).await.unwrap();
    assert!(
        bob_get_hook_res.status() == StatusCode::FORBIDDEN
            || bob_get_hook_res.status() == StatusCode::NOT_FOUND,
        "Bob must not read Alice's webhook; got {}",
        bob_get_hook_res.status()
    );

    // Bob attempts to list deliveries for Alice's webhook -> 404 / 403 Forbidden
    let bob_list_deliv_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/webhooks/{}/deliveries", webhook_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_list_deliv_res = router.clone().oneshot(bob_list_deliv_req).await.unwrap();
    assert!(
        bob_list_deliv_res.status() == StatusCode::FORBIDDEN
            || bob_list_deliv_res.status() == StatusCode::NOT_FOUND,
        "Bob must not list deliveries for Alice's webhook; got {}",
        bob_list_deliv_res.status()
    );

    // Bob attempts to delete Alice's webhook -> 404 / 403 Forbidden
    let bob_del_hook_req = Request::builder()
        .method("DELETE")
        .uri(format!("/api/v1/webhooks/{}", webhook_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_del_hook_res = router.clone().oneshot(bob_del_hook_req).await.unwrap();
    assert!(
        bob_del_hook_res.status() == StatusCode::FORBIDDEN
            || bob_del_hook_res.status() == StatusCode::NOT_FOUND,
        "Bob must not delete Alice's webhook; got {}",
        bob_del_hook_res.status()
    );

    // Alice successfully retrieves her webhook
    let alice_get_hook_req = Request::builder()
        .method("GET")
        .uri(format!("/api/v1/webhooks/{}", webhook_id))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .body(Body::empty())
        .unwrap();
    let alice_get_hook_res = router.clone().oneshot(alice_get_hook_req).await.unwrap();
    assert_eq!(alice_get_hook_res.status(), StatusCode::OK);

    // -------------------------------------------------------------------------
    // 8. Route Authorization: Reviews & Comments Cross-Tenant IDOR
    // -------------------------------------------------------------------------
    // Set up PR #1 in Alice's repo with a review and comment
    let pr_alice_record = PullRequest {
        id: "pr_alice_1_id".to_string(),
        repo_id: repo_alice.id.clone(),
        pr_number: 1,
        author_id: alice.id.clone(),
        title: "Feature branch".to_string(),
        body: "Feature description".to_string(),
        base_branch: "main".to_string(),
        head_branch: "feat".to_string(),
        base_commit_id: "0000000000000000000000000000000000000001".to_string(),
        head_commit_id: "0000000000000000000000000000000000000002".to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
        merge_commit_id: None,
        merge_strategy: None,
        merged_by_user_id: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        closed_at_nanos: None,
        merged_at_nanos: None,
    };
    db.create_pull_request(pr_alice_record).await.unwrap();

    let rev_alice = PullRequestReview {
        id: "rev_alice_id".to_string(),
        pr_id: "pr_alice_1_id".to_string(),
        reviewer_id: alice.id.clone(),
        commit_id: "0000000000000000000000000000000000000002".to_string(),
        state: "APPROVED".to_string(),
        body: "LGTM".to_string(),
        dismissed_by_user_id: None,
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.create_pr_review(rev_alice).await.unwrap();

    let prc_alice = PullRequestReviewComment {
        id: "prc_alice_id".to_string(),
        pr_id: "pr_alice_1_id".to_string(),
        review_id: Some("rev_alice_id".to_string()),
        author_id: alice.id.clone(),
        file_path: "src/main.rs".to_string(),
        commit_id: "0000000000000000000000000000000000000002".to_string(),
        diff_side: "RIGHT".to_string(),
        line_number: 10,
        body: "Consider renaming".to_string(),
        is_resolved: false,
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.create_pr_review_comment(prc_alice).await.unwrap();

    // Set up PR #1 in Bob's repo
    let pr_bob_record = PullRequest {
        id: "pr_bob_1_id".to_string(),
        repo_id: repo_bob.id.clone(),
        pr_number: 1,
        author_id: bob.id.clone(),
        title: "Bob Feature".to_string(),
        body: "Bob description".to_string(),
        base_branch: "main".to_string(),
        head_branch: "feat".to_string(),
        base_commit_id: "0000000000000000000000000000000000000001".to_string(),
        head_commit_id: "0000000000000000000000000000000000000002".to_string(),
        state: "OPEN".to_string(),
        is_draft: false,
        merge_commit_id: None,
        merge_strategy: None,
        merged_by_user_id: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        closed_at_nanos: None,
        merged_at_nanos: None,
    };
    db.create_pull_request(pr_bob_record).await.unwrap();

    // Bob attempts cross-tenant IDOR to dismiss Alice's review using his own repo context -> 404 Not Found
    let bob_dismiss_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/bob/public-repo/pulls/1/reviews/rev_alice_id/dismiss")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_dismiss_res = router.clone().oneshot(bob_dismiss_req).await.unwrap();
    assert_eq!(
        bob_dismiss_res.status(),
        StatusCode::NOT_FOUND,
        "Cross-PR review dismissal must fail with 404 Not Found"
    );

    // Bob attempts cross-tenant IDOR to resolve Alice's review comment using his own repo context -> 404 Not Found
    let bob_resolve_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/bob/public-repo/pulls/1/reviews/comments/prc_alice_id/resolve")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "resolved": true }).to_string()))
        .unwrap();
    let bob_resolve_res = router.clone().oneshot(bob_resolve_req).await.unwrap();
    assert_eq!(
        bob_resolve_res.status(),
        StatusCode::NOT_FOUND,
        "Cross-PR review comment resolution must fail with 404 Not Found"
    );

    // -------------------------------------------------------------------------
    // 9. Route Authorization: Discussions & Discovery Private Leakage Prevention
    // -------------------------------------------------------------------------
    // Anonymous user attempts to list discussion categories on Alice's private repo -> 404 Not Found
    let anon_cats_req = Request::builder()
        .method("GET")
        .uri("/api/v1/repos/alice/secret-project/discussions/categories")
        .body(Body::empty())
        .unwrap();
    let anon_cats_res = router.clone().oneshot(anon_cats_req).await.unwrap();
    assert_eq!(
        anon_cats_res.status(),
        StatusCode::NOT_FOUND,
        "Anonymous category access on private repo must fail with 404 Not Found"
    );

    // Anonymous user attempts to list topics on Alice's private repo -> 404 Not Found
    let anon_topics_req = Request::builder()
        .method("GET")
        .uri("/api/v1/repos/alice/secret-project/topics")
        .body(Body::empty())
        .unwrap();
    let anon_topics_res = router.clone().oneshot(anon_topics_req).await.unwrap();
    assert_eq!(
        anon_topics_res.status(),
        StatusCode::NOT_FOUND,
        "Anonymous topics access on private repo must fail with 404 Not Found"
    );

    // Anonymous user attempts to list languages on Alice's private repo -> 404 Not Found
    let anon_langs_req = Request::builder()
        .method("GET")
        .uri("/api/v1/repos/alice/secret-project/languages")
        .body(Body::empty())
        .unwrap();
    let anon_langs_res = router.clone().oneshot(anon_langs_req).await.unwrap();
    assert_eq!(
        anon_langs_res.status(),
        StatusCode::NOT_FOUND,
        "Anonymous languages access on private repo must fail with 404 Not Found"
    );

    // Anonymous user attempts to trigger language analysis on Alice's private repo -> 401 Unauthorized
    let anon_analyze_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-project/languages/analyze")
        .body(Body::empty())
        .unwrap();
    let anon_analyze_res = router.clone().oneshot(anon_analyze_req).await.unwrap();
    assert_eq!(anon_analyze_res.status(), StatusCode::UNAUTHORIZED);

    // Bob (non-member) attempts to trigger language analysis on Alice's private repo -> 404 / 403
    let bob_analyze_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/alice/secret-project/languages/analyze")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .body(Body::empty())
        .unwrap();
    let bob_analyze_res = router.clone().oneshot(bob_analyze_req).await.unwrap();
    assert!(
        bob_analyze_res.status() == StatusCode::FORBIDDEN
            || bob_analyze_res.status() == StatusCode::NOT_FOUND,
        "Bob must not trigger language analysis on Alice's private repo; got {}",
        bob_analyze_res.status()
    );

    // -------------------------------------------------------------------------
    // 10. Route Authorization: Security Center Cross-Tenant Remediation IDOR
    // -------------------------------------------------------------------------
    let finding_alice = SecretScanFinding {
        id: "finding_alice_id".to_string(),
        repo_id: repo_alice.id.clone(),
        commit_oid: "head".to_string(),
        file_path: "secret.key".to_string(),
        line_number: 1,
        rule_id: "spine-api-key".to_string(),
        rule_name: "Spine API Key".to_string(),
        severity: "CRITICAL".to_string(),
        safe_fingerprint: "safe_fp_1234".to_string(),
        status: "OPEN".to_string(),
        resolved_at_nanos: None,
        created_at_nanos: now,
    };
    db.create_secret_scan_finding(finding_alice).await.unwrap();

    // Bob attempts cross-tenant remediation of Alice's finding using his own repo context -> 404 Not Found
    let bob_remediate_req = Request::builder()
        .method("POST")
        .uri("/api/v1/repos/bob/public-repo/security/secrets/findings/finding_alice_id/remediate")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_bob_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(json!({ "status": "RESOLVED" }).to_string()))
        .unwrap();
    let bob_remediate_res = router.clone().oneshot(bob_remediate_req).await.unwrap();
    assert_eq!(
        bob_remediate_res.status(),
        StatusCode::NOT_FOUND,
        "Cross-tenant secret finding remediation must fail with 404 Not Found"
    );

    // -------------------------------------------------------------------------
    // 11. Release Asset Name Validation & Header Injection Prevention
    // -------------------------------------------------------------------------
    // 11a. Traversal sequence in asset name -> 400 Bad Request
    let traversal_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/secret-project/releases/{}/assets?name=../../etc/passwd",
            release_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"TRAVERSAL".to_vec()))
        .unwrap();
    let traversal_res = router.clone().oneshot(traversal_req).await.unwrap();
    assert_eq!(
        traversal_res.status(),
        StatusCode::BAD_REQUEST,
        "Directory traversal in asset name must fail with 400 Bad Request"
    );

    // 11b. Header injection attempt (CRLF) in asset name -> 400 Bad Request
    let crlf_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/secret-project/releases/{}/assets?name=evil%0d%0aInjected-Header:%20pwned",
            release_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"CRLF".to_vec()))
        .unwrap();
    let crlf_res = router.clone().oneshot(crlf_req).await.unwrap();
    assert_eq!(
        crlf_res.status(),
        StatusCode::BAD_REQUEST,
        "CRLF sequence in asset name must fail with 400 Bad Request"
    );

    // 11c. Quote characters in asset name -> 400 Bad Request
    let quote_req = Request::builder()
        .method("POST")
        .uri(format!(
            "/api/v1/repos/alice/secret-project/releases/{}/assets?name=exploit%22.bin",
            release_id
        ))
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_alice_plain))
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .body(Body::from(b"QUOTE".to_vec()))
        .unwrap();
    let quote_res = router.clone().oneshot(quote_req).await.unwrap();
    assert_eq!(
        quote_res.status(),
        StatusCode::BAD_REQUEST,
        "Quotes in asset name must fail with 400 Bad Request"
    );
}
