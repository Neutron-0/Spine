use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

use spine_api::{
    core::types::*,
    db::{Database, InMemoryDatabase},
    AppState,
};

#[tokio::test]
async fn test_permission_aware_global_and_code_search() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    // 1. Create User
    let alice = User {
        id: "user-alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        normalized_email: "alice@example.com".to_string(),
        display_name: "Alice Developer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(alice).await.unwrap();

    // 2. Create Public Repo with source code
    let public_repo = ForgeRepository {
        id: "repo-pub".to_string(),
        owner_user_id: "user-alice".to_string(),
        owner_username: "alice".to_string(),
        owner_org_id: None,
        name: "spine-core-pub".to_string(),
        normalized_name: "spine-core-pub".to_string(),
        description: "Public high-performance version control engine".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo-pub".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        archived: false,
    };
    db.create_repository(public_repo).await.unwrap();

    // Set topic
    state.discovery.set_repository_topics("user-alice", "repo-pub", &["distributed-systems".to_string(), "rust".to_string()]).await.unwrap();

    // Add source file to public repo
    let pub_code_dir = storage_root.join("repo-pub/src");
    fs::create_dir_all(&pub_code_dir).unwrap();
    fs::write(
        pub_code_dir.join("engine.rs"),
        "pub fn initialize_quantum_engine() {\n    println!(\"Superluminal VCS Ready\");\n}\n",
    )
    .unwrap();

    // 3. Create Private Repo with secret source code
    let private_repo = ForgeRepository {
        id: "repo-priv".to_string(),
        owner_user_id: "user-alice".to_string(),
        owner_username: "alice".to_string(),
        owner_org_id: None,
        name: "stealth-project".to_string(),
        normalized_name: "stealth-project".to_string(),
        description: "Secret internal research".to_string(),
        is_private: true,
        default_branch: "main".to_string(),
        native_repo_id: "repo-priv".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        archived: false,
    };
    db.create_repository(private_repo).await.unwrap();

    let priv_code_dir = storage_root.join("repo-priv/src");
    fs::create_dir_all(&priv_code_dir).unwrap();
    fs::write(
        priv_code_dir.join("stealth.rs"),
        "pub fn confidential_payload() {\n    // SUPER_SECRET_ALGORITHM_XYZ\n}\n",
    )
    .unwrap();

    // 4. Anonymous Search for "quantum"
    let anon_code_res = state
        .search
        .search("quantum", Some(SearchResultType::Code), None, None)
        .await
        .unwrap();

    assert_eq!(anon_code_res.total, 1);
    assert_eq!(anon_code_res.hits[0].repo_id, "repo-pub");
    assert_eq!(anon_code_res.hits[0].file_path.as_deref(), Some("src/engine.rs"));
    assert_eq!(anon_code_res.hits[0].line_number, Some(1));
    assert!(anon_code_res.hits[0].snippet.contains("initialize_quantum_engine"));

    // 5. Anonymous Search for secret payload should find 0 results (Permission Isolation)
    let anon_priv_res = state
        .search
        .search("SUPER_SECRET_ALGORITHM_XYZ", Some(SearchResultType::Code), None, None)
        .await
        .unwrap();
    assert_eq!(anon_priv_res.total, 0, "Anonymous search must not reveal private repo code");

    // 6. Authenticated Search by Alice for secret payload should succeed
    let alice_priv_res = state
        .search
        .search("SUPER_SECRET_ALGORITHM_XYZ", Some(SearchResultType::Code), None, Some("user-alice"))
        .await
        .unwrap();
    assert_eq!(alice_priv_res.total, 1, "Repository owner must be able to search private code");
    assert_eq!(alice_priv_res.hits[0].repo_id, "repo-priv");

    // 7. Search for repository by topic
    let topic_res = state
        .search
        .search("distributed-systems", Some(SearchResultType::Repository), None, None)
        .await
        .unwrap();
    assert_eq!(topic_res.total, 1);
    assert_eq!(topic_res.hits[0].repo_name, "alice/spine-core-pub");
}
