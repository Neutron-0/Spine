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

    // 2. Authenticated inspect attempt with directory traversal ('..') must be rejected with 400 Bad Request
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

    // 3. Authenticated inspect attempt outside approved root must be rejected with 403 Forbidden
    let req_outside = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/inspect")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "source_path": if cfg!(windows) { "C:\\Windows\\System32" } else { "/etc/shadow" }
        })).unwrap()))
        .unwrap();

    let res_outside = app.clone().oneshot(req_outside).await.unwrap();
    assert_eq!(res_outside.status(), StatusCode::FORBIDDEN);

    // 4. Authenticated run migration attempt with traversal in target_name must be rejected with 400 Bad Request
    let req_target_traversal = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/run")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "source_path": "valid_import_repo",
            "target_name": "../escaped_repo"
        })).unwrap()))
        .unwrap();

    let res_target = app.clone().oneshot(req_target_traversal).await.unwrap();
    assert_eq!(res_target.status(), StatusCode::BAD_REQUEST);

    // 5. Authenticated export_git without write permission or on non-existent repo must return 404/403
    let req_export_unauth = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/export")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "repo_name": "otheruser/secret-repo",
            "target_path": "exported_git"
        })).unwrap()))
        .unwrap();

    let res_export = app.clone().oneshot(req_export_unauth).await.unwrap();
    assert!(
        res_export.status() == StatusCode::NOT_FOUND
            || res_export.status() == StatusCode::FORBIDDEN
    );

    // 6. Authenticated export_git to path outside approved migration root must be rejected with 403 Forbidden
    let req_export_outside = Request::builder()
        .method("POST")
        .uri("/api/v1/migration/export")
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::COOKIE, format!("spine_session={}", session_token))
        .body(Body::from(serde_json::to_vec(&serde_json::json!({
            "repo_name": "my-repo",
            "target_path": if cfg!(windows) { "C:\\Windows\\System32\\export" } else { "/var/export" }
        })).unwrap()))
        .unwrap();

    let res_export_out = app.clone().oneshot(req_export_outside).await.unwrap();
    assert!(
        res_export_out.status() == StatusCode::FORBIDDEN
            || res_export_out.status() == StatusCode::NOT_FOUND
    );
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
    use spine_object_store::{ObjectStore, ObjectStoreError};

    let cyclic_id = ObjectId::from_hex("3333333333333333333333333333333333333333333333333333333333333333").unwrap();
    let entries = vec![
        TreeEntry {
            mode: FileMode::Directory,
            name: "self_loop".to_string(),
            object_id: cyclic_id,
        }
    ];
    let payload = serialize_tree(&entries);

    struct MockCyclicStore {
        cyclic_id: ObjectId,
        payload: Vec<u8>,
    }

    impl ObjectStore for MockCyclicStore {
        fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), ObjectStoreError> {
            if *id == self.cyclic_id {
                Ok((ObjectType::Tree, self.payload.clone()))
            } else {
                Err(ObjectStoreError::NotFound(*id))
            }
        }
        fn write_object(&self, _object_type: ObjectType, _payload: &[u8]) -> Result<ObjectId, ObjectStoreError> {
            Ok(self.cyclic_id)
        }
        fn has_object(&self, id: &ObjectId) -> bool {
            *id == self.cyclic_id
        }
        fn list_objects(&self) -> Result<Vec<ObjectId>, ObjectStoreError> {
            Ok(vec![self.cyclic_id])
        }
    }

    let store = MockCyclicStore {
        cyclic_id,
        payload,
    };

    let mut out = std::collections::BTreeMap::new();
    let result = spine_api::api::routes_git_http::flatten_tree_recursive(&store, &cyclic_id, "", &mut out);

    // It must return an Error detecting cyclic reference, NOT loop forever or stack overflow!
    assert!(result.is_err(), "Cyclic tree must return an error and not stack overflow");
    let err_str = result.err().unwrap().to_string();
    assert!(err_str.contains("Cyclic tree reference detected"), "Error should report cycle: {}", err_str);
}

#[tokio::test]
async fn test_tree_flattening_allows_shared_subtrees_in_valid_dag() {
    use spine_core::{FileMode, ObjectId, ObjectType, TreeEntry};
    use spine_format::serialize_tree;
    use spine_object_store::{FileSystemObjectStore, ObjectStore};

    let temp_dir = tempfile::tempdir().unwrap();
    let store = FileSystemObjectStore::new(temp_dir.path(), spine_core::HashAlgorithm::Blake3, 3).unwrap();

    // 1. Create a leaf blob and shared subtree
    let dummy_blob_id = ObjectId::from_hex("2222222222222222222222222222222222222222222222222222222222222222").unwrap();
    let shared_entries = vec![
        TreeEntry {
            mode: FileMode::Regular,
            name: "readme.txt".to_string(),
            object_id: dummy_blob_id,
        }
    ];
    let payload_shared = serialize_tree(&shared_entries);
    let shared_subtree_id = store.write_object(ObjectType::Tree, &payload_shared).unwrap();

    // 2. Create root tree containing TWO different directory entries ("dir_a" and "dir_b")
    // both pointing to the SAME shared_subtree_id (standard Git / CAS deduplication DAG)
    let root_entries = vec![
        TreeEntry {
            mode: FileMode::Directory,
            name: "dir_a".to_string(),
            object_id: shared_subtree_id,
        },
        TreeEntry {
            mode: FileMode::Directory,
            name: "dir_b".to_string(),
            object_id: shared_subtree_id,
        },
    ];
    let payload_root = serialize_tree(&root_entries);
    let root_tree_id = store.write_object(ObjectType::Tree, &payload_root).unwrap();

    // 3. Flatten tree via routes_git_http::flatten_tree_recursive
    let mut out_http = std::collections::BTreeMap::new();
    let res_http = spine_api::api::routes_git_http::flatten_tree_recursive(&store, &root_tree_id, "", &mut out_http);
    assert!(res_http.is_ok(), "Shared subtrees across sibling directories in a DAG must succeed: {:?}", res_http.err());
    assert!(out_http.contains_key("dir_a/readme.txt"), "dir_a/readme.txt must be flattened");
    assert!(out_http.contains_key("dir_b/readme.txt"), "dir_b/readme.txt must be flattened");

    // 4. Flatten tree via NativeVcsService::flatten_tree
    let vcs = spine_api::storage::vcs::NativeVcsService::new(temp_dir.path());
    let mut out_vcs = std::collections::BTreeMap::new();
    let res_vcs = vcs.flatten_tree(&store, &root_tree_id, "", &mut out_vcs);
    assert!(res_vcs.is_ok(), "NativeVcsService must successfully flatten shared subtrees: {:?}", res_vcs.err());
    assert!(out_vcs.contains_key("dir_a/readme.txt"));
    assert!(out_vcs.contains_key("dir_b/readme.txt"));

    // 5. Test collect_changed_paths_from_commits on commit with shared subtrees
    let commit = spine_core::Commit {
        format_version: 2,
        root_tree: root_tree_id,
        parents: vec![],
        author_name: "Dev".to_string(),
        author_email: "dev@example.com".to_string(),
        committer_name: "Dev".to_string(),
        committer_email: "dev@example.com".to_string(),
        timestamp_nanos: 0,
        message: "Initial commit with shared subtrees".to_string(),
    };
    let dummy_goid = spine_migration::git_format::oid::GitOid::zero();
    let new_commits = vec![(dummy_goid, ObjectId::from_bytes([0u8; 32]), commit)];
    let paths_res = spine_api::api::routes_git_http::collect_changed_paths_from_commits(&store, &new_commits);
    assert!(paths_res.is_ok(), "collect_changed_paths_from_commits must succeed: {:?}", paths_res.err());
    let paths = paths_res.unwrap();
    assert!(paths.contains(&"dir_a/readme.txt".to_string()));
    assert!(paths.contains(&"dir_b/readme.txt".to_string()));
}
