use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

use spine_api::{
    core::types::*,
    db::{Database, InMemoryDatabase},
    AppState,
};

#[tokio::test]
async fn test_developer_and_repository_discovery() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    // 1. Create User
    let charlie = User {
        id: "user-charlie".to_string(),
        username: "charlie".to_string(),
        normalized_username: "charlie".to_string(),
        email: "charlie@example.com".to_string(),
        normalized_email: "charlie@example.com".to_string(),
        display_name: "Charlie Distributed".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(charlie).await.unwrap();

    // 2. Extended Profile CRUD
    let profile = state
        .discovery
        .update_user_profile(
            "user-charlie",
            Some("Building distributed consensus algorithms"),
            Some("Spine Labs"),
            Some("San Francisco, CA"),
            Some("https://spine.dev"),
            Some("charlie_dist"),
            Some("# Welcome to Charlie's Profile\nSystems engineer working on VCS."),
            Some(vec!["repo-1".to_string()]),
        )
        .await
        .unwrap();

    assert_eq!(
        profile.bio.as_deref(),
        Some("Building distributed consensus algorithms")
    );
    assert_eq!(profile.company.as_deref(), Some("Spine Labs"));
    assert_eq!(profile.pinned_repo_ids, vec!["repo-1".to_string()]);

    let fetched = state.discovery.get_user_profile("charlie").await.unwrap();
    assert_eq!(fetched.location.as_deref(), Some("San Francisco, CA"));

    // 3. Create Repository & Setup Files for Language Analysis
    let repo = ForgeRepository {
        id: "repo-lang".to_string(),
        owner_user_id: "user-charlie".to_string(),
        owner_username: "charlie".to_string(),
        owner_org_id: None,
        name: "spine-hyperdrive".to_string(),
        normalized_name: "spine-hyperdrive".to_string(),
        description: "Polyglot distributed storage engine".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo-lang".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        archived: false,
    };
    db.create_repository(repo).await.unwrap();

    let repo_dir = storage_root.join("repo-lang");
    fs::create_dir_all(repo_dir.join("src")).unwrap();
    fs::create_dir_all(repo_dir.join("web")).unwrap();

    // 1000 bytes of Rust
    fs::write(repo_dir.join("src/main.rs"), "a".repeat(1000)).unwrap();
    // 500 bytes of TypeScript
    fs::write(repo_dir.join("web/index.ts"), "b".repeat(500)).unwrap();

    // 4. Run Language Analysis
    let languages = state
        .discovery
        .analyze_repository_languages("repo-lang")
        .await
        .unwrap();
    assert_eq!(languages.len(), 2);
    assert_eq!(languages[0].language, "Rust");
    assert_eq!(languages[0].bytes, 1000);
    assert!((languages[0].percentage - 66.66).abs() < 1.0);

    assert_eq!(languages[1].language, "TypeScript");
    assert_eq!(languages[1].bytes, 500);
    assert!((languages[1].percentage - 33.33).abs() < 1.0);

    // 5. Test Topic Tagging with Normalization
    let raw_topics = vec![
        "  RUST  ".to_string(),
        "DISTRIBUTED-SYSTEMS".to_string(),
        "INVALID$$$CHAR".to_string(), // Should be rejected by validation
        "storage".to_string(),
    ];
    let topics = state
        .discovery
        .set_repository_topics("user-charlie", "repo-lang", &raw_topics)
        .await
        .unwrap();

    let topic_names: Vec<String> = topics.into_iter().map(|t| t.topic).collect();
    assert!(topic_names.contains(&"rust".to_string()));
    assert!(topic_names.contains(&"distributed-systems".to_string()));
    assert!(topic_names.contains(&"storage".to_string()));
    assert!(!topic_names.contains(&"invalid$$$char".to_string()));

    // 6. Test Popular Topics
    let popular = state.discovery.list_popular_topics(10).await.unwrap();
    assert!(!popular.is_empty());
    assert!(popular
        .iter()
        .any(|p| p.topic == "rust" && p.repository_count >= 1));

    // 7. Test Explore Feed
    let explore = state
        .discovery
        .list_explore_repositories(Some("rust"))
        .await
        .unwrap();
    assert_eq!(explore.len(), 1);
    assert_eq!(explore[0].name, "spine-hyperdrive");
    assert_eq!(explore[0].primary_language.as_deref(), Some("Rust"));
}
