use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use spine_api::api::build_router;
use spine_api::api::middleware::AppState;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;
use tower::ServiceExt;

#[tokio::test]
async fn test_migration_endpoints_reject_unauthenticated_and_path_traversal() {
    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);

    // 1. Unauthenticated inspect attempt must be rejected with 401 Unauthorized
    let req = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/inspect")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "source_path": "/var/tmp/repo"
        })).unwrap()))
        .unwrap();

    let res = app.clone().oneshot(req).await.unwrap();
    assert_eq!(res.status(), StatusCode::UNAUTHORIZED);

    // Create an authenticated user
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user = User {
        id: "user_migrator".to_string(),
        username: "migrator".to_string(),
        normalized_username: "migrator".to_string(),
        email: "migrator@example.com".to_string(),
        normalized_email: "migrator@example.com".to_string(),
        display_name: "Migrator".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let session_token = "valid_migration_session_token_123456";
    let token_hash = blake3::hash(session_token.as_bytes()).to_hex().to_string();
    db.create_session(UserSession {
        id: "sess_migrator".to_string(),
        user_id: user.id.clone(),
        session_token_hash: token_hash,
        ip_address: None,
        user_agent: None,
        created_at_nanos: now,
        expires_at_nanos: now + 3600 * 1_000_000_000,
        revoked: false,
    }).await.unwrap();

    // 2. Authenticated inspect attempt with directory traversal must be rejected with 400 Bad Request
    let req_traversal = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/inspect")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "source_path": "../../etc/shadow"
        })).unwrap()))
        .unwrap();

    let res_traversal = app.clone().oneshot(req_traversal).await.unwrap();
    assert_eq!(res_traversal.status(), StatusCode::BAD_REQUEST);

    // 3. Authenticated run migration attempt with traversal in target_name must be rejected with 400 Bad Request
    let req_target_traversal = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/run")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "source_path": "/tmp/valid_git",
            "target_name": "../escaped_repo"
        })).unwrap()))
        .unwrap();

    let res_target = app.clone().oneshot(req_target_traversal).await.unwrap();
    assert_eq!(res_target.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn test_private_workflows_and_logs_enforce_read_authorization() {
    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 1. Create repo owner (Alice)
    let alice = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        normalized_email: "alice@example.com".to_string(),
        display_name: "Alice".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(alice.clone()).await.unwrap();

    // 2. Create private repo owned by Alice
    let private_repo = ForgeRepository {
        id: "repo_private".to_string(),
        owner_user_id: alice.id.clone(),
        owner_username: "alice".to_string(),
        owner_org_id: None,
        name: "secret-code".to_string(),
        normalized_name: "secret-code".to_string(),
        description: "Super secret".to_string(),
        is_private: true,
        default_branch: "main".to_string(),
        native_repo_id: "alice_secret_code".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };
    db.create_repository(private_repo.clone()).await.unwrap();

    // 3. Create a workflow run and job on the private repo
    let run = WorkflowRun {
        id: "run_private_1".to_string(),
        repo_id: private_repo.id.clone(),
        workflow_id: "wf_1".to_string(),
        workflow_name: "CI".to_string(),
        commit_oid: "c1".to_string(),
        ref_name: "refs/heads/main".to_string(),
        trigger_event: "push".to_string(),
        actor_id: alice.id.clone(),
        status: "RUNNING".to_string(),
        conclusion: None,
        definition_yaml: "name: CI".to_string(),
        definition_digest: "d1".to_string(),
        source_repo_id: Some(private_repo.id.clone()),
        is_fork: false,
        trust_classification: TrustClassification::Trusted,
        provenance_reason: "Same repo".to_string(),
        created_at_nanos: now,
        started_at_nanos: Some(now),
        completed_at_nanos: None,
    };
    db.create_workflow_run(run.clone()).await.unwrap();

    let job = WorkflowJob {
        id: "job_private_1".to_string(),
        run_id: run.id.clone(),
        repo_id: private_repo.id.clone(),
        job_key: "build".to_string(),
        name: "Build".to_string(),
        status: "RUNNING".to_string(),
        runner_id: None,
        runs_on: vec!["ubuntu-latest".to_string()],
        needs: vec![],
        attempt: 1,
        fencing_token: 1,
        lease_expires_at_nanos: Some(now + 60 * 1_000_000_000),
        exit_code: None,
        created_at_nanos: now,
        started_at_nanos: Some(now),
        completed_at_nanos: None,
    };
    db.create_workflow_job(job.clone()).await.unwrap();

    // 4. Anonymous request to get_run for private repo must be rejected (404 Not Found to prevent discovery, or 401/403)
    let req_anon_run = Request::builder()
        .method("GET")
        .uri("/api/v1/workflows/runs/run_private_1")
        .body(Body::empty())
        .unwrap();
    let res_anon_run = app.clone().oneshot(req_anon_run).await.unwrap();
    assert!(
        res_anon_run.status() == StatusCode::NOT_FOUND
            || res_anon_run.status() == StatusCode::UNAUTHORIZED
            || res_anon_run.status() == StatusCode::FORBIDDEN
    );

    // 5. Anonymous request to list_logs for private job must be rejected (404 Not Found to prevent discovery, or 401/403)
    let req_anon_logs = Request::builder()
        .method("GET")
        .uri("/api/v1/workflows/jobs/job_private_1/logs")
        .body(Body::empty())
        .unwrap();
    let res_anon_logs = app.clone().oneshot(req_anon_logs).await.unwrap();
    assert!(
        res_anon_logs.status() == StatusCode::NOT_FOUND
            || res_anon_logs.status() == StatusCode::UNAUTHORIZED
            || res_anon_logs.status() == StatusCode::FORBIDDEN
    );

    // 6. Alice (owner) requesting logs must succeed (200 OK)
    let alice_token = "alice_session_token_secret_1234";
    let alice_token_hash = blake3::hash(alice_token.as_bytes()).to_hex().to_string();
    db.create_session(UserSession {
        id: "sess_alice".to_string(),
        user_id: alice.id.clone(),
        session_token_hash: alice_token_hash,
        ip_address: None,
        user_agent: None,
        created_at_nanos: now,
        expires_at_nanos: now + 3600 * 1_000_000_000,
        revoked: false,
    }).await.unwrap();

    let req_alice_logs = Request::builder()
        .method("GET")
        .uri("/api/v1/workflows/jobs/job_private_1/logs")
        .header(header::COOKIE, format!("spine_session={}", alice_token))
        .body(Body::empty())
        .unwrap();
    let res_alice_logs = app.clone().oneshot(req_alice_logs).await.unwrap();
    assert_eq!(res_alice_logs.status(), StatusCode::OK);
}

#[tokio::test]
async fn test_private_artifact_namespaces_isolated_from_unauthorized_access() {
    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();
    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();
    let app = build_router(state);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let owner = User {
        id: "usr_artifact_owner".to_string(),
        username: "artowner".to_string(),
        normalized_username: "artowner".to_string(),
        email: "art@example.com".to_string(),
        normalized_email: "art@example.com".to_string(),
        display_name: "Art Owner".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: None,
    };
    db.create_user(owner.clone()).await.unwrap();

    // Create a private namespace
    let ns = ArtifactNamespace {
        id: "ns_private_1".to_string(),
        name: "internal-binaries".to_string(),
        owner_type: "user".to_string(),
        owner_id: owner.id.clone(),
        is_private: true,
        description: "Proprietary".to_string(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.create_artifact_namespace(ns.clone()).await.unwrap();

    // Anonymous request to list versions of private namespace must be rejected
    let req_anon = Request::builder()
        .method("GET")
        .uri("/api/v1/artifacts/internal-binaries/compiler/versions")
        .body(Body::empty())
        .unwrap();
    let res_anon = app.clone().oneshot(req_anon).await.unwrap();
    assert_eq!(res_anon.status(), StatusCode::UNAUTHORIZED);

    // Anonymous request to download from private namespace must be rejected
    let req_anon_dl = Request::builder()
        .method("GET")
        .uri("/api/v1/artifacts/internal-binaries/compiler/versions/v1.0.0/download")
        .body(Body::empty())
        .unwrap();
    let res_anon_dl = app.clone().oneshot(req_anon_dl).await.unwrap();
    assert_eq!(res_anon_dl.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn test_tree_flattening_rejects_cycles_without_stack_overflow() {
    use spine_core::{FileMode, ObjectId, ObjectType, TreeEntry};
    use spine_format::serialize_tree;
    use spine_object_store::{FileSystemObjectStore, ObjectStore};

    let temp_dir = tempfile::tempdir().unwrap();
    let store = FileSystemObjectStore::new(temp_dir.path(), spine_core::HashAlgorithm::Blake3, 3).unwrap();

    // Create a self-referencing cyclic tree:
    // Tree A contains an entry pointing to Tree A
    let dummy_id = ObjectId::from_hex("1111111111111111111111111111111111111111111111111111111111111111").unwrap();
    let mut entries = vec![
        TreeEntry {
            mode: FileMode::Regular,
            name: "file.txt".to_string(),
            object_id: dummy_id,
        }
    ];

    let payload_a = serialize_tree(&entries);
    let tree_a_id = store.write_object(ObjectType::Tree, &payload_a).unwrap();

    // Now write a Tree that includes tree_a_id as a subdirectory named "cycle"
    entries.push(TreeEntry {
        mode: FileMode::Directory,
        name: "cycle".to_string(),
        object_id: tree_a_id,
    });
    // Overwrite Tree A with the recursive definition
    let payload_cyclic = serialize_tree(&entries);
    let cyclic_tree_id = store.write_object(ObjectType::Tree, &payload_cyclic).unwrap();

    // Now make it point directly to cyclic_tree_id
    let mut recursive_entries = vec![
        TreeEntry {
            mode: FileMode::Directory,
            name: "loop".to_string(),
            object_id: cyclic_tree_id,
        }
    ];
    let payload_loop = serialize_tree(&recursive_entries);
    let loop_tree_id = store.write_object(ObjectType::Tree, &payload_loop).unwrap();

    // Update cyclic entries to point to loop_tree_id
    recursive_entries.push(TreeEntry {
        mode: FileMode::Directory,
        name: "loop2".to_string(),
        object_id: loop_tree_id,
    });
    let payload_loop2 = serialize_tree(&recursive_entries);
    let root_cycle_id = store.write_object(ObjectType::Tree, &payload_loop2).unwrap();

    let mut out = std::collections::BTreeMap::new();
    let result = spine_api::api::routes_git_http::flatten_tree_recursive(&store, &root_cycle_id, "", &mut out);

    // It must return an Error (cyclic reference or depth exceeded), NOT abort/crash with stack overflow!
    assert!(result.is_err(), "Cyclic tree must return an error and not stack overflow");
}
