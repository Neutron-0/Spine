use axum::body::Body;
use axum::http::{Request, StatusCode};
use spine_api::api::build_router;
use spine_api::api::middleware::AppState;
use spine_api::auth::mail::DevMailCatcher;
use spine_api::auth::rate_limit::RateLimiter;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::permissions::PermissionService;
use spine_api::repository::service::RepositoryService;
use spine_api::storage::vcs::NativeVcsService;
use spine_net::auth::{CapabilityTicket, Role, DEFAULT_SECRET_KEY};
use std::sync::Arc;
use tempfile::TempDir;
use tower::ServiceExt;

#[test]
fn test_capability_ticket_signing_and_verification() {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;
    let validity = 3600 * 1_000_000_000i64; // 1 hour

    let ticket = CapabilityTicket::new(
        "user_123".to_string(),
        "alice".to_string(),
        "repo_456".to_string(),
        "alice_myrepo".to_string(),
        Role::Contributor,
        validity,
        now,
    );

    let signed_str = ticket.encode_and_sign(DEFAULT_SECRET_KEY);
    assert!(!signed_str.is_empty());
    assert!(signed_str.contains('.'));

    // Verify valid ticket
    let verified = CapabilityTicket::decode_and_verify(&signed_str, DEFAULT_SECRET_KEY, now + 1000)
        .expect("Valid ticket should verify");
    assert_eq!(verified.user_id, "user_123");
    assert_eq!(verified.username, "alice");
    assert_eq!(verified.repo_id, "repo_456");
    assert_eq!(verified.native_repo_id, "alice_myrepo");
    assert_eq!(verified.role, Role::Contributor);

    // Tampered payload
    let parts: Vec<&str> = signed_str.split('.').collect();
    let tampered_payload = format!("{}a", parts[0]);
    let tampered_ticket = format!("{}.{}", tampered_payload, parts[1]);
    let err = CapabilityTicket::decode_and_verify(&tampered_ticket, DEFAULT_SECRET_KEY, now + 1000);
    assert!(err.is_err());

    // Expired ticket
    let expired_time = now + validity + 1_000_000_000i64; // past expiry
    let expired_err = CapabilityTicket::decode_and_verify(&signed_str, DEFAULT_SECRET_KEY, expired_time);
    assert!(expired_err.is_err());

    // Wrong secret key
    let wrong_key = b"wrong-secret-key-32-bytes-length";
    let wrong_err = CapabilityTicket::decode_and_verify(&signed_str, wrong_key, now + 1000);
    assert!(wrong_err.is_err());
}

#[tokio::test]
async fn test_issue_capability_ticket_api_route() {
    let tmp = TempDir::new().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let perms = Arc::new(PermissionService::new(db.clone()));
    let repos = Arc::new(RepositoryService::new(db.clone(), perms.clone()));
    let vcs = Arc::new(NativeVcsService::new(tmp.path().to_path_buf()));
    let rate_limiter = Arc::new(RateLimiter::new());
    let mail = Arc::new(DevMailCatcher::new());

    let artifact_store = Arc::new(spine_artifact::LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let artifacts = Arc::new(spine_api::artifact::ArtifactService::new(db.clone(), artifact_store));
    let releases = Arc::new(spine_api::releases::ReleaseService::new(db.clone(), artifacts.clone()));
    let packages = Arc::new(spine_api::packages::PackageService::new(db.clone(), artifacts.clone()));
    let cargo = Arc::new(spine_api::packages::CargoRegistryAdapter::new(packages.clone()));

    let state = AppState {
        db: db.clone(),
        permissions: perms,
        repositories: repos.clone(),
        vcs,
        rate_limiter,
        mail,
        coordinator: None,
        artifacts,
        releases,
        packages,
        cargo,
    };

    let router = build_router(state);

    // Create user and repository
    let user = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        normalized_email: "alice@example.com".to_string(),
        password_hash: "hash".to_string(),
        display_name: "Alice".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let repo = repos
        .create_repository(&user, "cool-project", Some("A test repo".into()), false, None)
        .await
        .unwrap();

    // 1. Issue capability ticket for public repo (authenticated)
    let body = serde_json::json!({
        "operation": "write"
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/repos/{}/{}/capability", user.username, repo.name))
        .header("Content-Type", "application/json")
        .extension(user.clone())
        .body(Body::from(body.to_string()))
        .unwrap();

    let resp = router.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let bytes = axum::body::to_bytes(resp.into_body(), 1024 * 64).await.unwrap();
    let val: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert!(val.get("ticket").is_some());
    assert_eq!(val["role"].as_str().unwrap(), "admin");
    assert_eq!(val["native_repo_id"].as_str().unwrap(), repo.native_repo_id);

    // 2. Private repo unauthorized caller returns 404 (prevent metadata leakage)
    let private_repo = repos
        .create_repository(&user, "secret-repo", None, true, None)
        .await
        .unwrap();

    let anon_req = Request::builder()
        .method("POST")
        .uri(format!("/api/v1/repos/{}/{}/capability", user.username, private_repo.name))
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();

    let anon_resp = router.oneshot(anon_req).await.unwrap();
    assert_eq!(anon_resp.status(), StatusCode::NOT_FOUND);
}
