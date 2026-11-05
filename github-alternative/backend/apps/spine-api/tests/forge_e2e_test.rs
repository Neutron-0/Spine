use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use serde_json::json;
use spine_api::api::{build_router, AppState};
use spine_api::auth::{DevMailCatcher, RateLimiter};
use spine_api::db::InMemoryDatabase;
use spine_api::permissions::PermissionService;
use spine_api::repository::{ProvisioningWorker, RepositoryService};
use spine_api::storage::NativeVcsService;
use std::sync::Arc;
use tempfile::tempdir;
use tower::ServiceExt;

#[tokio::test]
async fn test_forge_e2e_full_lifecycle() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();

    let db = Arc::new(InMemoryDatabase::new());
    let _mailer = Arc::new(DevMailCatcher::new());
    let permissions = Arc::new(PermissionService::new(db.clone()));
    let _repo_service = Arc::new(RepositoryService::new(db.clone(), permissions.clone()));
    let _vcs_service = Arc::new(NativeVcsService::new(storage_root.clone()));
    let _rate_limiter = Arc::new(RateLimiter::new());

    let artifact_store =
        Arc::new(spine_artifact::LocalArtifactStore::new(storage_root.join("artifacts")).unwrap());
    let artifacts = Arc::new(spine_api::artifact::ArtifactService::new(
        db.clone(),
        artifact_store,
    ));
    let _releases = Arc::new(spine_api::releases::ReleaseService::new(
        db.clone(),
        artifacts.clone(),
    ));
    let packages = Arc::new(spine_api::packages::PackageService::new(
        db.clone(),
        artifacts.clone(),
    ));
    let _cargo = Arc::new(spine_api::packages::CargoRegistryAdapter::new(
        packages.clone(),
    ));

    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let router = build_router(state);
    let worker = ProvisioningWorker::new(db.clone(), None, storage_root);

    // 1. Liveness & Readiness checks
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/health/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // 2. Register User Alice
    let register_payload = json!({
        "username": "alice",
        "email": "alice@spine.build",
        "password": "Password123!"
    });
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(register_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let reg_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let token = reg_json["token"]
        .as_str()
        .expect("Token returned on register")
        .to_string();

    // 3. Verify session
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/session")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let sess_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(sess_json["authenticated"], true);
    assert_eq!(sess_json["user"]["username"], "alice");

    // 4. Create Personal Access Token (PAT)
    let pat_payload = json!({
        "name": "CI-CD Token",
        "scopes": ["repo:read", "repo:write"]
    });
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/users/tokens")
                .header("authorization", format!("Bearer {}", token))
                .header("content-type", "application/json")
                .body(Body::from(pat_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let pat_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let raw_pat = pat_json["token"]
        .as_str()
        .expect("PAT token returned")
        .to_string();
    assert!(raw_pat.starts_with("sp_pat_"));

    // 5. Create a repository using PAT
    let repo_payload = json!({
        "name": "project-alpha",
        "description": "Test repository",
        "is_private": false,
        "durability": "ThreeCopies"
    });
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/repos")
                .header("x-spine-token", &raw_pat)
                .header("content-type", "application/json")
                .body(Body::from(repo_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let repo_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(repo_json["name"], "project-alpha");
    assert_eq!(repo_json["provisioning_status"], "PENDING");

    // 6. Background worker processes provisioning job
    let did_work = worker.process_next_job().await.unwrap();
    assert!(did_work);

    // 7. Get repository details
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/repos/alice/project-alpha")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let repo_detail: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(repo_detail["provisioning_status"], "READY");

    // 8. Create a private repository
    let priv_payload = json!({
        "name": "secret-vault",
        "description": "Confidential repository",
        "is_private": true,
        "durability": "ThreeCopies"
    });
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/repos")
                .header("authorization", format!("Bearer {}", token))
                .header("content-type", "application/json")
                .body(Body::from(priv_payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED);
    let _ = worker.process_next_job().await.unwrap();

    // 9. Register Bob and verify Bob cannot see or know about secret-vault
    let bob_register = json!({
        "username": "bob",
        "email": "bob@spine.build",
        "password": "Password123!"
    });
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/register")
                .header("content-type", "application/json")
                .body(Body::from(bob_register.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let bob_json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let bob_token = bob_json["token"].as_str().unwrap();

    // Bob requests Alice's private repository -> MUST return 404 Not Found!
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/repos/alice/secret-vault")
                .header("authorization", format!("Bearer {}", bob_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);

    // 10. Logout Alice
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/auth/logout")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);

    // Verifying Alice's session after logout -> authenticated = false
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/auth/session")
                .header("authorization", format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = to_bytes(res.into_body(), usize::MAX).await.unwrap();
    let sess_after: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(sess_after["authenticated"], false);
}
