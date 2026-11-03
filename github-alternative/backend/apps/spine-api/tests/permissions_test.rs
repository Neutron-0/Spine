use spine_api::core::error::ForgeError;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::permissions::PermissionService;
use spine_cluster::durability::DurabilityPolicy;
use std::sync::Arc;

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

fn make_test_repo(id: &str, owner_id: &str, owner_username: &str, name: &str, is_private: bool) -> ForgeRepository {
    ForgeRepository {
        id: id.to_string(),
        owner_user_id: owner_id.to_string(),
        owner_username: owner_username.to_string(),
        owner_org_id: None,
        name: name.to_string(),
        normalized_name: name.to_lowercase(),
        description: "Test repo".to_string(),
        is_private,
        default_branch: "main".to_string(),
        native_repo_id: format!("{}_{}", owner_username, name.to_lowercase()),
        durability_policy: DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        archived: false,
    }
}

#[tokio::test]
async fn test_public_repository_permissions() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = PermissionService::new(db.clone());

    let owner = make_test_user("usr_alice", "alice", "alice@spine.build");
    db.create_user(owner.clone()).await.unwrap();

    let public_repo = make_test_repo("repo_pub", &owner.id, &owner.username, "public-project", false);
    db.create_repository(public_repo.clone()).await.unwrap();

    let stranger = make_test_user("usr_bob", "bob", "bob@spine.build");
    db.create_user(stranger.clone()).await.unwrap();

    // Anonymous read: allowed (returns PermissionRole::Reader)
    assert!(perms.check_read_access(None, &public_repo).await.is_ok());

    // Anonymous write: requires auth (Unauthorized)
    match perms.check_write_access(None, &public_repo).await {
        Err(ForgeError::Unauthorized(_)) => {}
        other => panic!("Expected Unauthorized, got {:?}", other),
    }

    // Stranger read: allowed
    assert!(perms.check_read_access(Some(&stranger), &public_repo).await.is_ok());

    // Stranger write: forbidden (not collaborator)
    match perms.check_write_access(Some(&stranger), &public_repo).await {
        Err(ForgeError::Forbidden(_)) => {}
        other => panic!("Expected Forbidden, got {:?}", other),
    }

    // Owner read & write: allowed
    assert!(perms.check_read_access(Some(&owner), &public_repo).await.is_ok());
    assert!(perms.check_write_access(Some(&owner), &public_repo).await.is_ok());
}

#[tokio::test]
async fn test_private_repository_metadata_isolation() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = PermissionService::new(db.clone());

    let owner = make_test_user("usr_alice", "alice", "alice@spine.build");
    db.create_user(owner.clone()).await.unwrap();

    let private_repo = make_test_repo("repo_priv", &owner.id, &owner.username, "classified-project", true);
    db.create_repository(private_repo.clone()).await.unwrap();

    let stranger = make_test_user("usr_bob", "bob", "bob@spine.build");
    db.create_user(stranger.clone()).await.unwrap();

    // Anonymous read: must return NotFound (not Forbidden) to prevent existence leakage
    match perms.check_read_access(None, &private_repo).await {
        Err(ForgeError::NotFound(_)) => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }

    // Stranger read: must return NotFound
    match perms.check_read_access(Some(&stranger), &private_repo).await {
        Err(ForgeError::NotFound(_)) => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }

    // Stranger write: must return NotFound
    match perms.check_write_access(Some(&stranger), &private_repo).await {
        Err(ForgeError::NotFound(_)) => {}
        other => panic!("Expected NotFound, got {:?}", other),
    }

    // Owner read & write: allowed
    assert!(perms.check_read_access(Some(&owner), &private_repo).await.is_ok());
    assert!(perms.check_write_access(Some(&owner), &private_repo).await.is_ok());
}

#[tokio::test]
async fn test_collaborator_role_hierarchy() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let perms = PermissionService::new(db.clone());

    let owner = make_test_user("usr_owner", "owner", "owner@spine.build");
    db.create_user(owner.clone()).await.unwrap();

    let reader = make_test_user("usr_reader", "reader", "reader@spine.build");
    db.create_user(reader.clone()).await.unwrap();

    let writer = make_test_user("usr_writer", "writer", "writer@spine.build");
    db.create_user(writer.clone()).await.unwrap();

    let repo = make_test_repo("repo_team", &owner.id, &owner.username, "team-repo", true);
    db.create_repository(repo.clone()).await.unwrap();

    // Assign Reader permission
    db.create_permission(RepositoryPermission {
        id: "perm_1".to_string(),
        repo_id: repo.id.clone(),
        user_id: reader.id.clone(),
        role: PermissionRole::Reader,
        created_at_nanos: 100,
    })
    .await
    .unwrap();

    // Assign Collaborator (write) permission
    db.create_permission(RepositoryPermission {
        id: "perm_2".to_string(),
        repo_id: repo.id.clone(),
        user_id: writer.id.clone(),
        role: PermissionRole::Collaborator,
        created_at_nanos: 100,
    })
    .await
    .unwrap();

    // Reader tests:
    assert!(perms.check_read_access(Some(&reader), &repo).await.is_ok());
    assert!(perms.check_write_access(Some(&reader), &repo).await.is_err());
    assert!(perms.check_admin_access(Some(&reader), &repo).await.is_err());
    assert!(perms.check_delete_access(Some(&reader), &repo).await.is_err());

    // Writer tests:
    assert!(perms.check_read_access(Some(&writer), &repo).await.is_ok());
    assert!(perms.check_write_access(Some(&writer), &repo).await.is_ok());
    assert!(perms.check_admin_access(Some(&writer), &repo).await.is_err());
    assert!(perms.check_delete_access(Some(&writer), &repo).await.is_err());

    // Owner tests:
    assert!(perms.check_read_access(Some(&owner), &repo).await.is_ok());
    assert!(perms.check_write_access(Some(&owner), &repo).await.is_ok());
    assert!(perms.check_admin_access(Some(&owner), &repo).await.is_ok());
    assert!(perms.check_delete_access(Some(&owner), &repo).await.is_ok());
}
