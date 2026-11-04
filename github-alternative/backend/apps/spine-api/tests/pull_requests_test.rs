use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::storage::vcs::NativeVcsService;
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_commit, serialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

fn setup_test_repo_on_disk(tmp: &TempDir, native_repo_id: &str) -> (ObjectId, ObjectId, ObjectId) {
    let repo_dir = tmp.path().join("repos").join(native_repo_id);
    let spine_dir = repo_dir.join(".spine");
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("objects")).unwrap();

    let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();

    // 1. Initial commit (base): README.md
    let b1 = store
        .write_object(ObjectType::Blob, b"# Test Project\nInitial line\n")
        .unwrap();
    let tree_1_entries = vec![TreeEntry {
        mode: FileMode::Regular,
        name: "README.md".into(),
        object_id: b1,
    }];
    let tree_1 = store
        .write_object(ObjectType::Tree, &serialize_tree(&tree_1_entries))
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
    let c1_id = store
        .write_object(ObjectType::Commit, &serialize_commit(&c1))
        .unwrap();

    // 2. Commit on main branch: update README.md
    let b2 = store
        .write_object(
            ObjectType::Blob,
            b"# Test Project\nInitial line\nMain feature line\n",
        )
        .unwrap();
    let tree_2_entries = vec![TreeEntry {
        mode: FileMode::Regular,
        name: "README.md".into(),
        object_id: b2,
    }];
    let tree_2 = store
        .write_object(ObjectType::Tree, &serialize_tree(&tree_2_entries))
        .unwrap();

    let c2 = Commit::new(
        tree_2,
        vec![c1_id],
        "Alice".into(),
        "alice@spine.build".into(),
        "Alice".into(),
        "alice@spine.build".into(),
        2000,
        "Update on main".into(),
    );
    let c2_id = store
        .write_object(ObjectType::Commit, &serialize_commit(&c2))
        .unwrap();
    fs::write(
        spine_dir.join("refs").join("heads").join("main"),
        format!("{}\n", c2_id.to_hex()),
    )
    .unwrap();

    // 3. Commit on feature branch: add FEATURE.md
    let b3 = store
        .write_object(
            ObjectType::Blob,
            b"# New Feature\nDocumentation of feature\n",
        )
        .unwrap();
    let tree_3_entries = vec![
        TreeEntry {
            mode: FileMode::Regular,
            name: "README.md".into(),
            object_id: b2,
        },
        TreeEntry {
            mode: FileMode::Regular,
            name: "FEATURE.md".into(),
            object_id: b3,
        },
    ];
    let tree_3 = store
        .write_object(ObjectType::Tree, &serialize_tree(&tree_3_entries))
        .unwrap();

    let c3 = Commit::new(
        tree_3,
        vec![c2_id],
        "Bob".into(),
        "bob@spine.build".into(),
        "Bob".into(),
        "bob@spine.build".into(),
        3000,
        "Add feature branch".into(),
    );
    let c3_id = store
        .write_object(ObjectType::Commit, &serialize_commit(&c3))
        .unwrap();
    fs::write(
        spine_dir.join("refs").join("heads").join("feature"),
        format!("{}\n", c3_id.to_hex()),
    )
    .unwrap();

    (c1_id, c2_id, c3_id)
}

#[tokio::test]
async fn test_pull_request_diff_and_mergeability() {
    let tmp = TempDir::new().unwrap();
    let native_repo_id = "alice_project";
    let (_c1, _c2, c3) = setup_test_repo_on_disk(&tmp, native_repo_id);

    let vcs = NativeVcsService::new(tmp.path());
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());

    // 1. Create Pull Request
    let pr = PullRequest {
        id: "pr_1".into(),
        repo_id: "repo_1".into(),
        pr_number: 0,
        author_id: "usr_bob".into(),
        title: "Implement new feature".into(),
        body: "Closes #1 by adding FEATURE.md".into(),
        base_branch: "main".into(),
        head_branch: "feature".into(),
        base_commit_id: "".into(),
        head_commit_id: c3.to_hex(),
        state: "OPEN".into(),
        is_draft: false,
        merged_by_user_id: None,
        merge_commit_id: None,
        merge_strategy: Some("MERGE_COMMIT".into()),
        closed_at_nanos: None,
        merged_at_nanos: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
    };
    let created = db.create_pull_request(pr).await.unwrap();
    assert_eq!(created.pr_number, 1);

    // 2. Native Diff Computation
    let diff = vcs
        .compare_branches(native_repo_id, "main", "feature")
        .unwrap();
    assert_eq!(diff.files.len(), 1);
    assert_eq!(diff.files[0].path, "FEATURE.md");
    assert_eq!(diff.files[0].change_type, "ADDED");
    assert_eq!(diff.files[0].additions, 2);
    assert!(diff.files[0].patch.contains("+Documentation of feature"));
    assert_eq!(diff.commits_count, 1);

    // 3. Native Mergeability Check
    let report = vcs
        .check_mergeability(native_repo_id, "main", "feature")
        .unwrap();
    assert!(report.mergeable);
    assert_eq!(report.status, "CLEAN");
    assert!(report.conflict_files.is_empty());
}
