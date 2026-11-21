use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use serde_json::Value;
use spine_api::api::build_router;
use spine_api::api::middleware::AppState;
use spine_api::coordinator::client::CoordinatorClient;
use spine_api::db::InMemoryDatabase;
use spine_api::Database;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use tower::ServiceExt;

static TEST_MUTEX: Mutex<()> = Mutex::const_new(());

#[tokio::test]
async fn test_maintenance_barrier_endpoint_lifecycle_and_authorization() {
    let _guard = TEST_MUTEX.lock().await;
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

    let owner_token = acquire_json["owner_token"].as_str().unwrap();

    // 7. Release without auth fails
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);

    // 7b. Release with valid auth but missing owner_token fails with 403 Forbidden
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 7c. Release with valid auth but mismatched owner_token fails with 403 Forbidden
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"owner_token":"wrong-owner-token"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);

    // 7d. Diagnostics confirms barrier is STILL actively held after failed release attempts
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

    // 8. Release with valid admin token and matching owner_token succeeds
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-secret-admin-token-12345")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"owner_token":"{}"}}"#, owner_token)))
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

#[tokio::test]
async fn test_admin_authorization_username_denial() {
    let _guard = TEST_MUTEX.lock().await;
    std::env::remove_var("SPINE_ADMIN_TOKEN");
    std::env::remove_var("ADMIN_TOKEN");
    std::env::remove_var("SPINE_ADMIN_TOKEN_FILE");

    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 1. User 'admin-dev' with ordinary scopes (MUST NOT be treated as admin despite username containing 'admin')
    let user_admin_dev = spine_api::core::types::User {
        id: "usr_admin_dev".to_string(),
        username: "admin-dev".to_string(),
        normalized_username: "admin-dev".to_string(),
        email: "admin-dev@spineforge.local".to_string(),
        normalized_email: "admin-dev@spineforge.local".to_string(),
        display_name: "Developer with Admin in name".to_string(),
        password_hash: "hash_test".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user_admin_dev).await.unwrap();

    let pat_admin_dev_plain = "sp_pat_admin_dev_1234567890abcdef12345";
    let pat_admin_dev = spine_api::core::types::PersonalAccessToken {
        id: "pat_admin_dev".to_string(),
        user_id: "usr_admin_dev".to_string(),
        name: "dev_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_admin_dev_plain),
        scopes: vec!["repo:read".to_string(), "repo:write".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_admin_dev).await.unwrap();

    // 2. User 'notadmin' (contains neither admin nor root, standard user)
    let user_notadmin = spine_api::core::types::User {
        id: "usr_notadmin".to_string(),
        username: "notadmin".to_string(),
        normalized_username: "notadmin".to_string(),
        email: "notadmin@spineforge.local".to_string(),
        normalized_email: "notadmin@spineforge.local".to_string(),
        display_name: "Standard Dev".to_string(),
        password_hash: "hash_test".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user_notadmin).await.unwrap();

    let pat_notadmin_plain = "sp_pat_notadmin_1234567890abcdef12345";
    let pat_notadmin = spine_api::core::types::PersonalAccessToken {
        id: "pat_notadmin".to_string(),
        user_id: "usr_notadmin".to_string(),
        name: "notadmin_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_notadmin_plain),
        scopes: vec!["repo:read".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_notadmin).await.unwrap();

    // 3. User 'admin' but WITHOUT admin PAT scope (username is 'admin', but token only has 'repo')
    let user_plain_admin = spine_api::core::types::User {
        id: "usr_admin_plain".to_string(),
        username: "admin".to_string(),
        normalized_username: "admin".to_string(),
        email: "admin@spineforge.local".to_string(),
        normalized_email: "admin@spineforge.local".to_string(),
        display_name: "Admin Username Only".to_string(),
        password_hash: "hash_test".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user_plain_admin).await.unwrap();

    let pat_plain_admin_plain = "sp_pat_plain_admin_no_scope_12345678";
    let pat_plain_admin = spine_api::core::types::PersonalAccessToken {
        id: "pat_plain_admin".to_string(),
        user_id: "usr_admin_plain".to_string(),
        name: "unprivileged_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_plain_admin_plain),
        scopes: vec!["repo".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_plain_admin).await.unwrap();

    // 4. User 'ops-lead' with explicit 'admin' scope
    let user_ops_lead = spine_api::core::types::User {
        id: "usr_ops_lead".to_string(),
        username: "ops-lead".to_string(),
        normalized_username: "ops-lead".to_string(),
        email: "ops-lead@spineforge.local".to_string(),
        normalized_email: "ops-lead@spineforge.local".to_string(),
        display_name: "Real Cluster Operator".to_string(),
        password_hash: "hash_test".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user_ops_lead).await.unwrap();

    let pat_ops_lead_plain = "sp_pat_ops_lead_legit_admin_scope_123";
    let pat_ops_lead = spine_api::core::types::PersonalAccessToken {
        id: "pat_ops_lead".to_string(),
        user_id: "usr_ops_lead".to_string(),
        name: "ops_admin_token".to_string(),
        token_hash: spine_api::auth::pat::hash_pat(pat_ops_lead_plain),
        scopes: vec!["admin".to_string(), "repo".to_string()],
        expires_at_nanos: now + 86400 * 1_000_000_000,
        revoked: false,
        created_at_nanos: now,
        last_used_at_nanos: None,
    };
    db.create_pat(pat_ops_lead).await.unwrap();

    // Assert DENIAL for 'admin-dev'
    for uri in ["/api/v1/admin/maintenance/acquire", "/api/v1/admin/maintenance/release"] {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header(header::AUTHORIZATION, format!("Bearer {}", pat_admin_dev_plain))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"reason":"unauthorized attempt"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "User 'admin-dev' without admin scope must be denied on {}",
            uri
        );
    }

    // Assert DENIAL for 'notadmin'
    for uri in ["/api/v1/admin/maintenance/acquire", "/api/v1/admin/maintenance/release"] {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header(header::AUTHORIZATION, format!("Bearer {}", pat_notadmin_plain))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"reason":"unauthorized attempt"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "User 'notadmin' without admin scope must be denied on {}",
            uri
        );
    }

    // Assert DENIAL for user named 'admin' but lacking 'admin' scope
    for uri in ["/api/v1/admin/maintenance/acquire", "/api/v1/admin/maintenance/release"] {
        let req = Request::builder()
            .uri(uri)
            .method("POST")
            .header(header::AUTHORIZATION, format!("Bearer {}", pat_plain_admin_plain))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"reason":"unauthorized attempt"}"#))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::UNAUTHORIZED,
            "User 'admin' without admin scope must be denied on {}",
            uri
        );
    }

    // Assert SUCCESS for 'ops-lead' who possesses explicit 'admin' scope
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_ops_lead_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Authorized maintenance"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let acq_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let ops_owner_token = acq_json["owner_token"].as_str().unwrap();

    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_ops_lead_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"owner_token":"{}"}}"#, ops_owner_token)))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    // Revoke 'ops-lead' token -> must be denied with 401
    db.revoke_pat("pat_ops_lead", "usr_ops_lead").await.unwrap();
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, format!("Bearer {}", pat_ops_lead_plain))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Revoked attempt"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[allow(dead_code)]
struct MockParticipantServer {
    pub addr: String,
    pub in_maintenance: Arc<AtomicBool>,
    pub fencing_generation: Arc<AtomicU64>,
    pub fail_enter: Arc<AtomicBool>,
    pub fail_exit: Arc<AtomicBool>,
    pub enter_count: Arc<AtomicU64>,
    pub exit_count: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
}

impl Drop for MockParticipantServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}

async fn spawn_mock_participant(fail_enter: bool, fail_exit: bool) -> MockParticipantServer {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let in_maintenance = Arc::new(AtomicBool::new(false));
    let fencing_generation = Arc::new(AtomicU64::new(0));
    let fail_enter = Arc::new(AtomicBool::new(fail_enter));
    let fail_exit = Arc::new(AtomicBool::new(fail_exit));
    let enter_count = Arc::new(AtomicU64::new(0));
    let exit_count = Arc::new(AtomicU64::new(0));
    let stop = Arc::new(AtomicBool::new(false));

    let in_maint_c = Arc::clone(&in_maintenance);
    let fgen_c = Arc::clone(&fencing_generation);
    let f_enter_c = Arc::clone(&fail_enter);
    let f_exit_c = Arc::clone(&fail_exit);
    let ent_c = Arc::clone(&enter_count);
    let ext_c = Arc::clone(&exit_count);
    let stop_c = Arc::clone(&stop);

    tokio::spawn(async move {
        while !stop_c.load(Ordering::SeqCst) {
            if let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(Duration::from_millis(50), listener.accept()).await
            {
                let in_maint = Arc::clone(&in_maint_c);
                let fgen = Arc::clone(&fgen_c);
                let fe = Arc::clone(&f_enter_c);
                let fx = Arc::clone(&f_exit_c);
                let ec = Arc::clone(&ent_c);
                let xc = Arc::clone(&ext_c);

                tokio::spawn(async move {
                    let mut len_buf = [0u8; 4];
                    if stream.read_exact(&mut len_buf).await.is_err() {
                        return;
                    }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    let mut body = vec![0u8; len];
                    if stream.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let Ok(req) = ClusterRpcRequest::from_bytes(&body) else {
                        return;
                    };
                    let resp = match req {
                        ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation } => {
                            ec.fetch_add(1, Ordering::SeqCst);
                            if fe.load(Ordering::SeqCst) {
                                ClusterRpcResponse::Error("Injected failure on enter maintenance".to_string())
                            } else {
                                in_maint.store(true, Ordering::SeqCst);
                                fgen.store(fencing_generation, Ordering::SeqCst);
                                ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                            }
                        }
                        ClusterRpcRequest::ExitMaintenanceBarrier { fencing_generation } => {
                            xc.fetch_add(1, Ordering::SeqCst);
                            if fx.load(Ordering::SeqCst) {
                                ClusterRpcResponse::Error("Injected failure on exit maintenance".to_string())
                            } else {
                                in_maint.store(false, Ordering::SeqCst);
                                fgen.store(fencing_generation, Ordering::SeqCst);
                                ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                            }
                        }
                        _ => ClusterRpcResponse::Error("Unsupported mock RPC".to_string()),
                    };
                    if let Ok(rb) = resp.to_bytes() {
                        let _ = stream.write_all(&(rb.len() as u32).to_be_bytes()).await;
                        let _ = stream.write_all(&rb).await;
                        let _ = stream.flush().await;
                    }
                });
            }
        }
    });

    MockParticipantServer {
        addr,
        in_maintenance,
        fencing_generation,
        fail_enter,
        fail_exit,
        enter_count,
        exit_count,
        stop,
    }
}

#[tokio::test]
async fn test_maintenance_acquire_storage_node_failure_compensates_earlier_participants() {
    let _guard = TEST_MUTEX.lock().await;
    std::env::set_var("SPINE_ADMIN_TOKEN", "test-admin-token");

    let coord = spawn_mock_participant(false, false).await;
    let node1 = spawn_mock_participant(false, false).await;
    let node2 = spawn_mock_participant(true, false).await; // Injected enter failure!

    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    state.coordinator = Some(CoordinatorClient::new(coord.addr.clone()));
    state.storage_nodes = vec![
        ("node-1".to_string(), node1.addr.clone()),
        ("node-2".to_string(), node2.addr.clone()),
    ];

    let consistency_barrier = Arc::clone(&state.consistency_barrier);
    let active_maint = Arc::clone(&state.active_maintenance);
    let app = build_router(state);

    // Call acquire -> node-2 will fail, which must compensate node-1 and coordinator
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Test compensation"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::INTERNAL_SERVER_ERROR,
        "Acquisition must fail closed when a participant fails to enter"
    );

    // Wait a brief moment for RPC rollback rounds to finish
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify coordinator was compensated and is NOT in maintenance
    assert_eq!(
        coord.in_maintenance.load(Ordering::SeqCst),
        false,
        "Coordinator must have been exited from maintenance via compensation"
    );
    assert_eq!(coord.enter_count.load(Ordering::SeqCst), 1);
    assert_eq!(coord.exit_count.load(Ordering::SeqCst), 1);

    // Verify node-1 was compensated and is NOT in maintenance
    assert_eq!(
        node1.in_maintenance.load(Ordering::SeqCst),
        false,
        "Node 1 must have been exited from maintenance via compensation"
    );
    assert_eq!(node1.enter_count.load(Ordering::SeqCst), 1);
    assert_eq!(node1.exit_count.load(Ordering::SeqCst), 1);

    // Verify node-2 was never entered
    assert_eq!(
        node2.in_maintenance.load(Ordering::SeqCst),
        false,
        "Node 2 must not be in maintenance"
    );

    // Verify local barrier is NOT held
    assert_eq!(
        consistency_barrier.is_in_maintenance(),
        false,
        "Local consistency barrier must have been released on failure"
    );
    assert!(
        active_maint.lock().await.is_none(),
        "Active maintenance guard must be None after failure compensation"
    );
}

#[tokio::test]
async fn test_maintenance_release_participant_failure_reports_degraded_error() {
    let _guard = TEST_MUTEX.lock().await;
    std::env::set_var("SPINE_ADMIN_TOKEN", "test-admin-token");

    let coord = spawn_mock_participant(false, false).await;
    let node1 = spawn_mock_participant(false, true).await; // Injected exit failure!

    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    state.coordinator = Some(CoordinatorClient::new(coord.addr.clone()));
    state.storage_nodes = vec![("node-1".to_string(), node1.addr.clone())];

    let app = build_router(state);

    // Acquire succeeds
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Test release failure"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let acq_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    let owner_token = acq_json["owner_token"].as_str().unwrap();

    // Release fails because node1 fails to exit (mismatched token would fail earlier with 403, here valid token passes ownership gate)
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-token")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(r#"{{"owner_token":"{}"}}"#, owner_token)))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();

    assert_eq!(
        resp.status(),
        StatusCode::SERVICE_UNAVAILABLE,
        "Release must fail and report degraded/failed status if any participant fails to exit"
    );

    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let err_str = String::from_utf8_lossy(&body_bytes);
    assert!(
        err_str.contains("Cluster maintenance release failed or degraded"),
        "Error message must specify degraded release status: {}",
        err_str
    );
    assert!(
        err_str.contains("node-1"),
        "Error message must name the failed participant node: {}",
        err_str
    );
}

#[test]
fn test_bootstrap_admin_secret_suppression() {
    let temp_dir = tempfile::tempdir().unwrap();
    let token_file = temp_dir.path().join("admin_token");
    let password_file = temp_dir.path().join("admin_password");

    let bin_path = env!("CARGO_BIN_EXE_spine-api");
    let output = std::process::Command::new(bin_path)
        .arg("bootstrap-admin")
        .arg("--username")
        .arg("testbootstrap")
        .arg("--email")
        .arg("testbootstrap@spineforge.local")
        .arg("--token-out-file")
        .arg(&token_file)
        .arg("--password-out-file")
        .arg(&password_file)
        .arg("--in-memory-db")
        .arg("--profile")
        .arg("test")
        .output()
        .expect("Failed to execute spine-api binary");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        output.status.success(),
        "Bootstrap admin process failed: stdout: {}, stderr: {}",
        stdout,
        stderr
    );

    // Verify token and password files exist and are not empty
    assert!(token_file.exists(), "Token file was not created");
    assert!(password_file.exists(), "Password file was not created");

    let raw_token = std::fs::read_to_string(&token_file).unwrap();
    let raw_token = raw_token.trim();
    assert!(
        raw_token.starts_with("sp_pat_"),
        "Generated PAT has invalid format: {}",
        raw_token
    );

    let raw_password = std::fs::read_to_string(&password_file).unwrap();
    let raw_password = raw_password.trim();
    assert_eq!(
        raw_password.len(),
        32,
        "Generated password length should be 32 chars: {}",
        raw_password
    );

    // CRITICAL: Assert neither secret appears anywhere in captured output!
    assert!(
        !stdout.contains(raw_token),
        "CRITICAL LEAK: Admin token appeared in stdout!"
    );
    assert!(
        !stdout.contains(raw_password),
        "CRITICAL LEAK: Admin password appeared in stdout!"
    );
    assert!(
        !stderr.contains(raw_token),
        "CRITICAL LEAK: Admin token appeared in stderr!"
    );
    assert!(
        !stderr.contains(raw_password),
        "CRITICAL LEAK: Admin password appeared in stderr!"
    );

    // Assert file delivery banner is present
    assert!(stdout.contains("SUPPRESSED FROM STDOUT"));
    assert!(stdout.contains("Token File:"));
    assert!(stdout.contains("Password File:"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let token_meta = std::fs::metadata(&token_file).unwrap();
        assert_eq!(token_meta.permissions().mode() & 0o777, 0o600);
        let pass_meta = std::fs::metadata(&password_file).unwrap();
        assert_eq!(pass_meta.permissions().mode() & 0o777, 0o600);
    }
}

#[tokio::test]
async fn test_barrier_ownership_and_unacquired_backup_cleanup_isolation() {
    let _guard = TEST_MUTEX.lock().await;
    std::env::set_var("SPINE_ADMIN_TOKEN", "test-admin-secret-ownership-123");

    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);

    // 1. Operation A acquires maintenance barrier
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/acquire")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-secret-ownership-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"reason":"Operation A migration","timeout_seconds":300}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let acq_json: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(acq_json["status"], "acquired");
    let original_generation = acq_json["fencing_generation"].as_u64().unwrap();
    let original_owner_token = acq_json["owner_token"].as_str().unwrap().to_string();

    // Verify diagnostics: active maintenance held at original_generation
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], true);
    assert_eq!(diag["current_fencing_generation"], original_generation);

    // 2. Mismatched owner token CANNOT release Operation A's barrier
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-secret-ownership-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"owner_token":"impostor-operation-token-999"}"#))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Mismatched owner token must be rejected with 403 Forbidden"
    );

    // Missing owner token CANNOT release Operation A's barrier
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-secret-ownership-123")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::FORBIDDEN,
        "Missing owner token must be rejected with 403 Forbidden"
    );

    // Diagnostics confirms barrier is still actively held by Operation A
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], true);
    assert_eq!(diag["current_fencing_generation"], original_generation);

    // 3. Bind a local HTTP test server backed by the exact same AppState
    // with recording middleware to PROVE the backup script makes zero release requests.
    let release_endpoint_call_count = Arc::new(AtomicUsize::new(0));
    let release_count_middleware = release_endpoint_call_count.clone();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("Failed to bind ephemeral listener for isolated test server");
    let server_port = listener.local_addr().unwrap().port();
    let server_url = format!("http://127.0.0.1:{}", server_port);

    let recorded_app = app.clone().layer(axum::middleware::from_fn(
        move |req: axum::extract::Request, next: axum::middleware::Next| {
            let path = req.uri().path().to_string();
            let count = release_count_middleware.clone();
            async move {
                if path == "/api/v1/admin/maintenance/release" {
                    count.fetch_add(1, Ordering::SeqCst);
                }
                next.run(req).await
            }
        },
    ));

    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
    let server_task = tokio::spawn(async move {
        let _ = axum::serve(listener, recorded_app)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await;
    });

    // Run production local-only backup script invocation that fails before acquisition.
    // It must exit 1, and its cleanup trap must NOT make any release request.
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut workspace_root = manifest_dir;
    while let Some(parent) = workspace_root.parent() {
        if parent.join("scripts").join("backup_oob.sh").exists() {
            workspace_root = parent;
            break;
        }
        workspace_root = parent;
    }
    let script_path = workspace_root.join("scripts").join("backup_oob.sh");
    assert!(
        script_path.exists(),
        "scripts/backup_oob.sh must exist at {:?}",
        script_path
    );

    // Provide a temporary passphrase file for deterministic execution inside workspace
    let temp_pass_file = workspace_root.join(".test_passphrase_barrier.tmp");
    std::fs::write(
        &temp_pass_file,
        "test-backup-encryption-passphrase-at-least-32-chars-entropy!\n",
    )
    .expect("Failed to write temporary passphrase file");

    // Fail the test if the script or Bash cannot run instead of silently skipping assertions.
    let script_str = script_path.to_str().unwrap().replace('\\', "/");
    let pass_path_str = temp_pass_file.to_str().unwrap().replace('\\', "/");

    let runner_script = format!(
        r#"
set -euo pipefail
S="{}"
P="{}"
if [ ! -f "$S" ]; then
  if command -v wslpath >/dev/null 2>&1; then S=$(wslpath -u "$S"); elif command -v cygpath >/dev/null 2>&1; then S=$(cygpath -u "$S"); fi
fi
if [ ! -f "$P" ]; then
  if command -v wslpath >/dev/null 2>&1; then P=$(wslpath -u "$P"); elif command -v cygpath >/dev/null 2>&1; then P=$(cygpath -u "$P"); fi
fi

# Set up recording curl stub to log all curl invocations
STUB_DIR=$(mktemp -d /tmp/curl_stub_XXXXXX 2>/dev/null || mktemp -d "${{TMPDIR:-/tmp}}/curl_stub_XXXXXX")
CURL_LOG="${{STUB_DIR}}/curl_calls.log"
touch "${{CURL_LOG}}"
REAL_CURL=$(command -v curl || echo "/usr/bin/curl")

cat <<EOF > "${{STUB_DIR}}/curl"
#!/usr/bin/env bash
echo "\$@" >> "${{CURL_LOG}}"
exec "${{REAL_CURL}}" "\$@"
EOF
chmod +x "${{STUB_DIR}}/curl"
export PATH="${{STUB_DIR}}:${{PATH}}"

export MSYS_NO_PATHCONV=1
cd "$(dirname "$S")/.."
export SPINE_API_URL="{}"
export SPINE_DEPLOYMENT_PROFILE=core-forge-v1
export SPINE_BACKUP_REQUIRE_REMOTE=0
export SPINE_ADMIN_TOKEN=test-admin-secret-ownership-123
export SPINE_BACKUP_PASSPHRASE_FILE="$P"

set +e
bash "$S"
EXIT_CODE=$?
set -e

# Echo recorded curl calls into stderr for test assertions
echo "=== RECORDED CURL CALLS START ===" >&2
cat "${{CURL_LOG}}" >&2
echo "=== RECORDED CURL CALLS END ===" >&2
rm -rf "${{STUB_DIR}}"
exit ${{EXIT_CODE}}
"#,
        script_str, pass_path_str, server_url
    );

    let mut child = std::process::Command::new("bash")
        .arg("-s")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("Failed to spawn bash for backup_oob.sh: bash must be available and runnable");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(runner_script.as_bytes()).expect("Failed to write to bash stdin");
    }
    let output = child.wait_with_output().expect("Failed to wait on bash child");

    let _ = std::fs::remove_file(&temp_pass_file);

    assert!(
        !output.status.success(),
        "Production local-only backup must fail closed with exit code 1"
    );
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    assert!(
        stderr_str.contains("Local-only backup (SPINE_BACKUP_REQUIRE_REMOTE=0) is strictly FORBIDDEN"),
        "Stderr must contain policy rejection: {}",
        stderr_str
    );

    // PROOF: Assert recording curl stub captured zero calls to release endpoint
    assert!(
        !stderr_str.contains("maintenance/release"),
        "Recording curl stub proved a release request was made! Stderr: {}",
        stderr_str
    );
    // PROOF: Assert local HTTP server backed by AppState received ZERO release requests from failed backup
    assert_eq!(
        release_endpoint_call_count.load(Ordering::SeqCst),
        0,
        "Local HTTP server backed by AppState must have received ZERO release requests from unacquired backup"
    );

    // Must NOT contain cleanup release attempt for barrier it did not acquire
    assert!(
        !stderr_str.contains("releasing acquired cluster maintenance barrier"),
        "Unacquired backup must not attempt to release barrier: {}",
        stderr_str
    );
    assert!(
        !stdout_str.contains("releasing acquired cluster maintenance barrier"),
        "Unacquired backup must not attempt to release barrier in stdout: {}",
        stdout_str
    );
    // CRITICAL: Owner token must never appear anywhere in captured stdout or stderr
    assert!(
        !stdout_str.contains(&original_owner_token),
        "CRITICAL LEAK: Owner token appeared in stdout: {}",
        stdout_str
    );
    assert!(
        !stderr_str.contains(&original_owner_token),
        "CRITICAL LEAK: Owner token appeared in stderr: {}",
        stderr_str
    );
    assert!(
        !stdout_str.contains("owner_token"),
        "CRITICAL LEAK: raw 'owner_token' key appeared in stdout: {}",
        stdout_str
    );

    // Diagnostics confirms barrier is STILL actively held by Operation A with unchanged generation
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(
        diag["in_maintenance"], true,
        "Operation A's barrier must remain active after failed unacquired backup"
    );
    assert_eq!(
        diag["current_fencing_generation"], original_generation,
        "Fencing generation must remain completely unchanged"
    );

    // 4. Operation A cleanly releases its own barrier with its matching owner_token
    let req = Request::builder()
        .uri("/api/v1/admin/maintenance/release")
        .method("POST")
        .header(header::AUTHORIZATION, "Bearer test-admin-secret-ownership-123")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(format!(
            r#"{{"owner_token":"{}"}}"#,
            original_owner_token
        )))
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "Matching owner token must release the barrier"
    );

    // Shutdown isolated background test server
    let _ = shutdown_tx.send(());
    let _ = server_task.await;

    // Diagnostics confirms maintenance is now released
    let req = Request::builder()
        .uri("/health/diagnostics")
        .method("GET")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    let body_bytes = axum::body::to_bytes(resp.into_body(), 1024 * 1024).await.unwrap();
    let diag: Value = serde_json::from_slice(&body_bytes).unwrap();
    assert_eq!(diag["in_maintenance"], false);
}

#[test]
fn test_backup_script_acquire_output_redacts_owner_token() {
    let manifest_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut workspace_root = manifest_dir;
    while let Some(parent) = workspace_root.parent() {
        if parent.join("scripts").join("backup_oob.sh").exists() {
            workspace_root = parent;
            break;
        }
        workspace_root = parent;
    }
    let script_path = workspace_root.join("scripts").join("backup_oob.sh");
    assert!(
        script_path.exists(),
        "scripts/backup_oob.sh must exist at {:?}",
        script_path
    );
    let content = std::fs::read_to_string(&script_path).expect("failed to read backup_oob.sh");

    // Ensure raw ACQUIRE_RESP is not printed anywhere in the script
    assert!(
        !content.contains("echo \"Maintenance barrier acquired: ${ACQUIRE_RESP}\""),
        "CRITICAL LEAK: backup_oob.sh must not echo raw ACQUIRE_RESP containing owner_token"
    );
    assert!(
        !content.contains("echo ${ACQUIRE_RESP}"),
        "CRITICAL LEAK: backup_oob.sh must not echo $ACQUIRE_RESP"
    );

    // Execute the exact acquire logging logic in Bash and verify stdout and stderr
    let test_token = "secret_barrier_owner_token_999888777";
    let test_script = format!(
        r#"
set -euo pipefail
ACQUIRE_RESP='{{"status":"acquired","fencing_generation":105,"owner_token":"{}","lease_expires_at_nanos":17777777777}}'
BARRIER_OWNER_TOKEN=$(echo "${{ACQUIRE_RESP}}" | grep -o '"owner_token":"[^"]*"' | cut -d'"' -f4 || true)
if [ -z "${{BARRIER_OWNER_TOKEN}}" ]; then
  exit 1
fi
BARRIER_ACQUIRED=1
ACQUIRE_STATUS=$(echo "${{ACQUIRE_RESP}}" | grep -o '"status":"[^"]*"' | cut -d'"' -f4 || echo "acquired")
ACQUIRE_FENCING_GEN=$(echo "${{ACQUIRE_RESP}}" | grep -o '"fencing_generation":[0-9]*' | cut -d: -f2 || echo "unknown")
echo "Maintenance barrier acquired: status=${{ACQUIRE_STATUS}}, fencing_generation=${{ACQUIRE_FENCING_GEN}}"
"#,
        test_token
    );

    let mut child = std::process::Command::new("bash")
        .arg("-s")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("bash must run");

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(test_script.as_bytes()).expect("failed to write test_script to bash stdin");
    }
    let output = child.wait_with_output().expect("failed to wait on bash child");

    if !output.status.success() {
        panic!(
            "Bash execution failed with code {:?}. Stderr: {}, Stdout: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr),
            String::from_utf8_lossy(&output.stdout)
        );
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("Maintenance barrier acquired: status=acquired, fencing_generation=105"),
        "Stdout must log status and fencing generation: {}",
        stdout
    );
    assert!(
        !stdout.contains(test_token),
        "CRITICAL LEAK: Owner token appeared in stdout: {}",
        stdout
    );
    assert!(
        !stderr.contains(test_token),
        "CRITICAL LEAK: Owner token appeared in stderr: {}",
        stderr
    );
    assert!(
        !stdout.contains("owner_token"),
        "Raw field name owner_token should not appear in log output: {}",
        stdout
    );
}
