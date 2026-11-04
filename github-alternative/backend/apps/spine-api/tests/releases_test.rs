use spine_api::artifact::ArtifactService;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::releases::ReleaseService;
use spine_artifact::LocalArtifactStore;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_release_and_asset_lifecycle() {
    let tmp = tempdir().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let store = Arc::new(LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let artifacts = Arc::new(ArtifactService::new(db.clone(), store));
    let releases = ReleaseService::new(db.clone(), artifacts.clone());

    // 1. Setup user & repository
    let user = User {
        id: "usr_alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@spine.build".to_string(),
        normalized_email: "alice@spine.build".to_string(),
        display_name: "Alice Engineer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(user.clone()).await.unwrap();

    let repo = ForgeRepository {
        id: "repo_core".to_string(),
        owner_user_id: user.id.clone(),
        owner_username: user.username.clone(),
        owner_org_id: None,
        name: "spine-core".to_string(),
        normalized_name: "spine-core".to_string(),
        description: "Spine VCS Core Engine".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::ThreeCopies,
        provisioning_status: ProvisioningStatus::Ready,
        native_repo_id: "repo_core_native".to_string(),
        archived: false,
        created_at_nanos: 1,
        updated_at_nanos: 1,
    };
    db.create_repository(repo.clone()).await.unwrap();

    // 2. Create release v1.0.0
    let req = CreateReleaseRequest {
        tag_name: "v1.0.0".to_string(),
        target_commit_id: "commit_sha_123456789".to_string(),
        title: "Spine v1.0.0 Official Release".to_string(),
        body: "First major production release of the Spine engine.".to_string(),
        is_draft: Some(false),
        is_prerelease: Some(false),
    };

    let release = releases
        .create_release(&repo.id, &user.id, req)
        .await
        .expect("Release creation should succeed");

    assert_eq!(release.tag_name, "v1.0.0");
    assert_eq!(release.title, "Spine v1.0.0 Official Release");
    assert_eq!(release.author.username, "alice");
    assert!(!release.is_draft);
    assert!(!release.is_prerelease);

    // 3. Prevent duplicate tag release
    let dup_req = CreateReleaseRequest {
        tag_name: "v1.0.0".to_string(),
        target_commit_id: "commit_sha_other".to_string(),
        title: "Duplicate".to_string(),
        body: "".to_string(),
        is_draft: Some(false),
        is_prerelease: Some(false),
    };
    let dup_err = releases.create_release(&repo.id, &user.id, dup_req).await;
    assert!(dup_err.is_err());

    // 4. Upload binary asset to release (e.g. spine-x86_64-pc-windows-msvc.zip)
    let binary_data = vec![0x50, 0x4B, 0x03, 0x04, 0x14, 0x00, 0x08, 0x00]; // Zip signature
    let asset = releases
        .upload_asset(
            &release.id,
            "spine-x86_64-pc-windows-msvc.zip",
            "application/zip",
            &binary_data,
        )
        .await
        .expect("Asset upload should succeed");

    assert_eq!(asset.name, "spine-x86_64-pc-windows-msvc.zip");
    assert_eq!(asset.content_type, "application/zip");
    assert_eq!(asset.size_bytes, binary_data.len() as i64);
    assert_eq!(asset.downloads_count, 0);

    // 5. Download asset and verify downloads counter incremented
    let (downloaded_asset, bytes) = releases
        .download_asset(&asset.id)
        .await
        .expect("Asset download should succeed");

    assert_eq!(bytes, binary_data);
    assert_eq!(downloaded_asset.downloads_count, 1);

    // 6. List releases
    let list = releases.list_releases(&repo.id, true).await.unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].assets.len(), 1);
    assert_eq!(list[0].assets[0].name, "spine-x86_64-pc-windows-msvc.zip");

    // 7. Delete asset and release
    releases.delete_asset(&asset.id).await.unwrap();
    let updated_rel = releases.get_release(&release.id).await.unwrap();
    assert_eq!(updated_rel.assets.len(), 0);

    releases.delete_release(&release.id).await.unwrap();
    assert!(releases.get_release(&release.id).await.is_err());
}
