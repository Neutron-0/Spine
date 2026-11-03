use spine_api::storage::NativeVcsService;
use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_commit, serialize_tree};
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;

fn setup_test_spine_repo(root: &PathBuf, native_repo_id: &str) -> (PathBuf, ObjectId, ObjectId) {
    let repo_dir = root.join("repos").join(native_repo_id);
    let spine_dir = repo_dir.join(".spine");
    fs::create_dir_all(spine_dir.join("objects")).unwrap();
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("refs").join("tags")).unwrap();

    let store = FileSystemObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();

    // 1. Create a blob for README.md
    let readme_content = b"# Spine Forge Repository\n\nNative Rust VCS hosting.\n";
    let blob_id = store.write_object(ObjectType::Blob, readme_content).unwrap();

    // 2. Create a tree containing README.md
    let tree_entries = vec![TreeEntry {
        mode: FileMode::Regular,
        name: "README.md".to_string(),
        object_id: blob_id,
    }];
    let tree_data = serialize_tree(&tree_entries);
    let tree_id = store.write_object(ObjectType::Tree, &tree_data).unwrap();

    // 3. Create initial commit
    let commit = Commit::new(
        tree_id,
        vec![],
        "Alice Developer".to_string(),
        "alice@spine.build".to_string(),
        "Alice Developer".to_string(),
        "alice@spine.build".to_string(),
        1772580000_000_000_000,
        "Initial commit\n".to_string(),
    );
    let commit_data = serialize_commit(&commit);
    let commit_id = store.write_object(ObjectType::Commit, &commit_data).unwrap();

    // 4. Write HEAD and refs
    fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        spine_dir.join("refs").join("heads").join("main"),
        format!("{}\n", commit_id),
    )
    .unwrap();
    fs::write(
        spine_dir.join("refs").join("heads").join("feature-test"),
        format!("{}\n", commit_id),
    )
    .unwrap();
    fs::write(
        spine_dir.join("refs").join("tags").join("v1.0.0"),
        format!("{}\n", commit_id),
    )
    .unwrap();

    (repo_dir, commit_id, blob_id)
}

#[tokio::test]
async fn test_native_vcs_browsing_branches_and_tags() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();
    let native_repo_id = "alice_my-repo";
    let (_repo_dir, commit_id, _blob_id) = setup_test_spine_repo(&storage_root, native_repo_id);

    let vcs = NativeVcsService::new(storage_root);

    // List branches
    let branches = vcs.list_branches(native_repo_id).expect("Branches listed");
    assert_eq!(branches.len(), 2);

    let main_branch = branches
        .iter()
        .find(|b| b.name == "main")
        .expect("Main branch exists");
    assert_eq!(main_branch.commit_id, commit_id.to_hex());

    let feature_branch = branches
        .iter()
        .find(|b| b.name == "feature-test")
        .expect("Feature branch exists");
    assert_eq!(feature_branch.commit_id, commit_id.to_hex());

    // List tags
    let tags = vcs.list_tags(native_repo_id).expect("Tags listed");
    assert_eq!(tags.len(), 1);
    assert_eq!(tags[0].name, "v1.0.0");
    assert_eq!(tags[0].target_id, commit_id.to_hex());
}

#[tokio::test]
async fn test_native_vcs_commits_tree_and_blob() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().to_path_buf();
    let native_repo_id = "alice_my-repo";
    let (_repo_dir, commit_id, blob_id) = setup_test_spine_repo(&storage_root, native_repo_id);

    let vcs = NativeVcsService::new(storage_root);

    // Get commits
    let commits = vcs
        .get_commits(native_repo_id, Some("main"), 10)
        .expect("Commits listed");
    assert_eq!(commits.len(), 1);
    assert_eq!(commits[0].id, commit_id.to_hex());
    assert!(commits[0].message.contains("Initial commit"));
    assert_eq!(commits[0].author_name, "Alice Developer");

    // Inspect tree at root
    let tree_entries = vcs
        .get_tree(native_repo_id, "main", None)
        .expect("Tree inspected");
    assert_eq!(tree_entries.len(), 1);
    assert_eq!(tree_entries[0].name, "README.md");
    assert!(!tree_entries[0].is_dir);
    assert_eq!(tree_entries[0].object_id, blob_id.to_hex());

    // Inspect blob content
    let blob = vcs
        .get_blob(native_repo_id, "main", "README.md")
        .expect("Blob fetched");
    assert_eq!(blob.object_id, blob_id.to_hex());
    assert!(!blob.is_binary);
    assert!(blob.content.unwrap().contains("# Spine Forge Repository"));
}
