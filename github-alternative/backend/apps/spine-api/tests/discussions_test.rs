use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

use spine_api::{
    core::types::*,
    db::{Database, InMemoryDatabase},
    AppState,
};

#[tokio::test]
async fn test_discussions_lifecycle_and_moderation() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let alice = User {
        id: "user-alice".to_string(),
        username: "alice".to_string(),
        normalized_username: "alice".to_string(),
        email: "alice@example.com".to_string(),
        normalized_email: "alice@example.com".to_string(),
        display_name: "Alice Maintainer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(alice).await.unwrap();

    let bob = User {
        id: "user-bob".to_string(),
        username: "bob".to_string(),
        normalized_username: "bob".to_string(),
        email: "bob@example.com".to_string(),
        normalized_email: "bob@example.com".to_string(),
        display_name: "Bob Contributor".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(bob).await.unwrap();

    let repo = ForgeRepository {
        id: "repo-disc".to_string(),
        owner_user_id: "user-alice".to_string(),
        owner_username: "alice".to_string(),
        owner_org_id: None,
        name: "community-repo".to_string(),
        normalized_name: "community-repo".to_string(),
        description: "Community discussion testing".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo-disc".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        archived: false,
    };
    db.create_repository(repo).await.unwrap();

    // 1. Verify default categories exist (e.g. Q&A, Announcements, General)
    let categories = state
        .discussions
        .list_categories("repo-disc")
        .await
        .unwrap();
    assert!(!categories.is_empty());
    let qa_cat = categories
        .iter()
        .find(|c| c.slug == "q-and-a")
        .expect("Q&A category must exist");
    assert!(qa_cat.is_answerable);

    // 2. Bob creates a discussion in Q&A
    let disc = state
        .discussions
        .create_discussion(
            "user-bob",
            "repo-disc",
            &qa_cat.id,
            "How does Spine handle CRDT-based merging?",
            "Can someone explain the 3-way merge resolution algorithm?",
        )
        .await
        .unwrap();

    assert_eq!(disc.number, 1);
    assert_eq!(disc.author.username, "bob");
    assert!(!disc.is_pinned);
    assert!(!disc.is_locked);
    assert!(disc.answered_comment_id.is_none());

    // 3. Alice provides an answer comment
    let answer_comment = state
        .discussions
        .create_comment(
            "user-alice",
            "repo-disc",
            1,
            None,
            "Spine uses three-way AST merge with conflict matrix journal replay.",
        )
        .await
        .unwrap();

    assert_eq!(answer_comment.author.username, "alice");

    // 4. Bob marks Alice's comment as the accepted answer
    let answered_disc = state
        .discussions
        .mark_answer("user-bob", "repo-disc", 1, &answer_comment.id)
        .await
        .unwrap();

    assert_eq!(
        answered_disc.answered_comment_id,
        Some(answer_comment.id.clone())
    );

    // 5. Test reaction toggle on the comment
    let added = state
        .discussions
        .toggle_reaction("user-bob", "COMMENT", &answer_comment.id, "+1")
        .await
        .unwrap();
    assert!(added, "Reaction should be added");

    let comments = state
        .discussions
        .list_comments(Some("user-bob"), "repo-disc", 1)
        .await
        .unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].reactions.len(), 1);
    assert_eq!(comments[0].reactions[0].reaction, "+1");
    assert_eq!(comments[0].reactions[0].count, 1);

    // Toggle reaction off
    let removed = state
        .discussions
        .toggle_reaction("user-bob", "COMMENT", &answer_comment.id, "+1")
        .await
        .unwrap();
    assert!(!removed, "Reaction should be removed upon toggle");

    // 6. Moderation: Alice (admin) pins the discussion
    let pinned_disc = state
        .discussions
        .update_discussion(
            "user-alice",
            "repo-disc",
            1,
            None,
            None,
            None,
            Some(true),
            None,
        )
        .await
        .unwrap();
    assert!(pinned_disc.is_pinned);

    // 7. Non-admin Bob attempts to lock the discussion (must fail)
    let lock_attempt = state
        .discussions
        .update_discussion(
            "user-bob",
            "repo-disc",
            1,
            None,
            None,
            None,
            None,
            Some(true),
        )
        .await;
    assert!(lock_attempt.is_err(), "Non-admin cannot lock discussions");
}
