use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::permissions::PermissionService;
use spine_api::repository::{ProvisioningWorker, RepositoryService};
use spine_cluster::durability::DurabilityPolicy;
use std::sync::Arc;
use tempfile::tempdir;

fn make_test_user(id: &str, username: &str, email: &str) -> User {
    User {
        id: id.to_string(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: email.to_string(),
        normalized_email: email.to_lowercase(),
        display_name: username.to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    }
}

#[tokio::test]
async fn test_repository_name_validation() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = Arc::new(PermissionService::new(db.clone()));
    let service = RepositoryService::new(db.clone(), perms);

    let user = make_test_user("usr_val", "val_user", "val@spine.build");
    db.create_user(user.clone()).await.unwrap();

    // Invalid names
    assert!(service.create_repository(&user, "", None, false, None).await.is_err());
    assert!(service.create_repository(&user, "repo/with/slash", None, false, None).await.is_err());
    assert!(service.create_repository(&user, "repo..dots", None, false, None).await.is_err());
    assert!(service.create_repository(&user, "-leading-dash", None, false, None).await.is_err());
    assert!(service.create_repository(&user, "repo space", None, false, None).await.is_err());

    // Valid names
    assert!(service.create_repository(&user, "valid-repo", None, false, None).await.is_ok());
    assert!(service.create_repository(&user, "valid_repo_2", None, false, None).await.is_ok());
    assert!(service.create_repository(&user, "Project123", None, false, None).await.is_ok());
}

#[tokio::test]
async fn test_provisioning_worker_job_execution() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = Arc::new(PermissionService::new(db.clone()));
    let service = RepositoryService::new(db.clone(), perms);
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();

    let worker = ProvisioningWorker::new(db.clone(), None, storage_root.clone());

    let user = make_test_user("usr_prov", "prov_user", "prov@spine.build");
    db.create_user(user.clone()).await.unwrap();

    // Create repository via service
    let repo = service
        .create_repository(
            &user,
            "my-spine-repo",
            Some("Test desc".to_string()),
            false,
            Some(DurabilityPolicy::ThreeCopies),
        )
        .await
        .expect("Repository creation succeeds");

    assert_eq!(repo.name, "my-spine-repo");
    assert_eq!(repo.provisioning_status, ProvisioningStatus::Pending);

    // Verify outbox job was queued
    let job = db.get_next_pending_job().await.unwrap().expect("Pending job found");
    assert_eq!(job.repo_id, repo.id);
    assert_eq!(job.status, ProvisioningStatus::Pending);

    // Worker executes one job pass
    let processed = worker.process_next_job().await.expect("Worker processes job");
    assert!(processed);

    // Verify repository is now READY
    let updated_repo = db.get_repository_by_id(&repo.id).await.unwrap().expect("Repo found");
    assert_eq!(updated_repo.provisioning_status, ProvisioningStatus::Ready);

    // Verify native .spine repository files on disk
    let repo_dir = storage_root.join("repos").join(&repo.native_repo_id);
    let spine_dir = repo_dir.join(".spine");
    assert!(spine_dir.exists(), ".spine directory should exist on disk");
    assert!(spine_dir.join("objects").exists(), "objects/ should exist");
    assert!(spine_dir.join("refs").join("heads").exists(), "refs/heads/ should exist");
    assert!(spine_dir.join("HEAD").exists(), "HEAD file should exist");
    assert!(spine_dir.join("config").exists(), "config file should exist");

    let head_content = std::fs::read_to_string(spine_dir.join("HEAD")).unwrap();
    assert_eq!(head_content.trim(), "ref: refs/heads/main");

    // No more jobs to process
    let processed_again = worker.process_next_job().await.unwrap();
    assert!(!processed_again);
}
