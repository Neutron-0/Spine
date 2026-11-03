use spine_api::storage::vcs::NativeVcsService;
use spine_cluster::write_pipeline::PrProtectionContext;
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_commit, serialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::fs;
use tempfile::TempDir;

fn setup_repo_for_merge(tmp: &TempDir, native_repo_id: &str) -> (ObjectId, ObjectId) {
    let repo_dir = tmp.path().join("repos").join(native_repo_id);
    let spine_dir = repo_dir.join(".spine");
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("objects")).unwrap();

    let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();

    // 1. Initial base commit
    let b1 = store.write_object(ObjectType::Blob, b"base content\n").unwrap();
    let tree_1 = store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[TreeEntry {
                mode: FileMode::Regular,
                name: "file.txt".into(),
                object_id: b1,
            }]),
        )
        .unwrap();

    let c1 = Commit::new(
        tree_1,
        vec![],
        "Alice".into(),
        "alice@spine.build".into(),
        "Alice".into(),
        "alice@spine.build".into(),
        1000,
        "Initial commit".into(),
    );
    let c1_id = store.write_object(ObjectType::Commit, &serialize_commit(&c1)).unwrap();
    fs::write(spine_dir.join("refs").join("heads").join("main"), format!("{}\n", c1_id.to_hex())).unwrap();

    // 2. Feature branch commit
    let b2 = store.write_object(ObjectType::Blob, b"feature content\n").unwrap();
    let tree_2 = store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[
                TreeEntry {
                    mode: FileMode::Regular,
                    name: "file.txt".into(),
                    object_id: b1,
                },
                TreeEntry {
                    mode: FileMode::Regular,
                    name: "new_feature.txt".into(),
                    object_id: b2,
                },
            ]),
        )
        .unwrap();

    let c2 = Commit::new(
        tree_2,
        vec![c1_id],
        "Bob".into(),
        "bob@spine.build".into(),
        "Bob".into(),
        "bob@spine.build".into(),
        2000,
        "Add new feature".into(),
    );
    let c2_id = store.write_object(ObjectType::Commit, &serialize_commit(&c2)).unwrap();
    fs::write(spine_dir.join("refs").join("heads").join("feature"), format!("{}\n", c2_id.to_hex())).unwrap();

    (c1_id, c2_id)
}

#[tokio::test]
async fn test_authoritative_pr_merge_pipeline() {
    let tmp = TempDir::new().unwrap();
    let native_repo_id = "alice_testrepo";
    let (base_id, head_id) = setup_repo_for_merge(&tmp, native_repo_id);

    let vcs = NativeVcsService::new(tmp.path().to_path_buf());

    let pr_ctx = PrProtectionContext {
        pr_number: 1,
        author_id: "usr_bob".to_string(),
        approved_count: 1,
        has_active_change_requests: false,
        has_unresolved_comments: false,
    };

    // Execute merge through Authoritative distributed write pipeline
    let result = vcs
        .execute_merge(
            native_repo_id,
            "main",
            "feature",
            "usr_alice",
            "Alice",
            "alice@spine.build",
            "Merge pull request #1 from feature",
            Some(&pr_ctx),
        )
        .await
        .expect("PR merge through authoritative pipeline should succeed");

    assert!(result.success);
    let merge_commit_hex = result.merge_commit_id.expect("Should have merge commit ID");
    assert!(!merge_commit_hex.is_empty());

    // Verify main branch now points to the new merge commit
    let new_main_oid = vcs.resolve_ref(native_repo_id, "main").unwrap();
    assert_eq!(new_main_oid.to_hex(), merge_commit_hex);

    // Verify merge commit parents
    let merge_commit = vcs.get_commit(native_repo_id, &merge_commit_hex).unwrap();
    assert_eq!(merge_commit.parents.len(), 2);
    assert_eq!(merge_commit.parents[0], base_id.to_hex());
    assert_eq!(merge_commit.parents[1], head_id.to_hex());

    // Verify durability status can be queried
    let durability_report = vcs.verify_durability(native_repo_id).await.unwrap();
    assert_eq!(durability_report.repo_id, native_repo_id);
    assert!(durability_report.is_durability_satisfied);
}

#[tokio::test]
async fn test_authoritative_merge_rejects_active_changes_requested() {
    let tmp = TempDir::new().unwrap();
    let native_repo_id = "alice_pr_rejected";
    let (_base_id, _head_id) = setup_repo_for_merge(&tmp, native_repo_id);

    let vcs = NativeVcsService::new(tmp.path().to_path_buf());

    let pr_ctx = PrProtectionContext {
        pr_number: 2,
        author_id: "usr_bob".to_string(),
        approved_count: 1,
        has_active_change_requests: true, // Changes requested!
        has_unresolved_comments: false,
    };

    let result = vcs
        .execute_merge(
            native_repo_id,
            "main",
            "feature",
            "usr_alice",
            "Alice",
            "alice@spine.build",
            "Merge pull request #2 from feature",
            Some(&pr_ctx),
        )
        .await;

    assert!(result.is_err(), "PR merge must be rejected when changes are requested");
    let err_str = result.err().unwrap().to_string();
    assert!(err_str.contains("CHANGES_REQUESTED") || err_str.contains("rejected"));
}
