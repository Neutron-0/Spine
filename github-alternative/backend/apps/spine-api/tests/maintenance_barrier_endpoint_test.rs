use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use spine_api::api::build_router;
use spine_api::api::middleware::AppState;
use spine_api::db::InMemoryDatabase;
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_maintenance_barrier_endpoint_lifecycle_and_authorization() {
    std::env::set_var("SPINE_ADMIN_TOKEN", "test-secret-admin-token-12345");

    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);

    // 1. Diagnostics initially reports normal state
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], false);
    assert_eq!(diag["active_writers"], 0);
    assert!(diag["current_fencing_generation"].as_u64().unwrap() >= 1);

    // 2. Unauthenticated acquire attempt must fail with 401
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"test backup"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 3. Invalid admin token must fail with 401
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer wrong-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"test backup"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 4. Valid admin token acquires barrier successfully
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Scheduled OOB Backup","timeout_seconds":120}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let acquire_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(acquire_json["status"], "acquired");
    let gen = acquire_json["fencing_generation"].as_u64().unwrap();
    assert!(gen >= 2);
    assert!(!acquire_json["owner_token"].as_str().unwrap().is_empty());

    // 5. Diagnostics now reports in_maintenance = true
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], true);
    assert_eq!(diag["current_fencing_generation"], gen);

    // 6. Conflicting acquire request returns 409 Conflict
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Concurrent backup"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);

    // 7. Release without auth fails
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 8. Release with valid admin token succeeds
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let release_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(release_json["status"], "released");

    // 9. Diagnostics returns to in_maintenance = false
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], false);
}
