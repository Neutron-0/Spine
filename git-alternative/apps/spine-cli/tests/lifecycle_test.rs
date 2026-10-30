use spine_config::SpineConfig;
use spine_core::{Commit, HashAlgorithm, ObjectType};
use spine_history::walk_history;
use spine_index::Index;
use spine_integrity::run_fsck;
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use spine_worktree::{compute_status, stage_path, write_index_to_tree, IgnoreRules};
use std::fs;
use tempfile::tempdir;

#[test]
fn test_end_to_end_repository_lifecycle() {
    let temp = tempdir().unwrap();
    let root = temp.path();
    let spine_dir = root.join(".spine");

    // 1. Initialize repository (.spine/)
    fs::create_dir_all(&spine_dir).unwrap();
    fs::create_dir_all(spine_dir.join("objects")).unwrap();
    fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
    fs::create_dir_all(spine_dir.join("tmp")).unwrap();

    let mut config = SpineConfig::default();
    config.set_value("user.name", "Test Developer").unwrap();
    config.set_value("user.email", "test@spine.org").unwrap();
    config.save_to_file(spine_dir.join("spine.toml")).unwrap();

    fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

    let store = FileSystemObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();
    let mut index = Index::new();
    // 2. Create working directory files (including nested and unicode)
    fs::write(root.join("README.md"), "# Hello Spine VCS\n").unwrap();

    let src_dir = root.join("src");
    fs::create_dir_all(&src_dir).unwrap();
    fs::write(
        src_dir.join("main.rs"),
        "fn main() { println!(\"Spine\"); }\n",
    )
    .unwrap();

    let docs_dir = root.join("docs");
    fs::create_dir_all(&docs_dir).unwrap();
    fs::write(
        docs_dir.join("unicode_文档.txt"),
        "Unicode UTF-8 verification\n",
    )
    .unwrap();

    // Create an ignored file
    fs::write(root.join(".spineignore"), "*.log\nignored_dir/\n").unwrap();
    fs::write(root.join("debug.log"), "This must be ignored\n").unwrap();

    let ignore = IgnoreRules::load_from_dir(root);

    // 3. Stage files
    let staged_count = stage_path(root, root, &mut index, &store, &ignore).unwrap();
    assert!(staged_count >= 3);
    index.save_to_file(spine_dir.join("index")).unwrap();

    // Verify .log file was not staged
    assert!(index.get("debug.log").is_none());
    assert!(index.get("README.md").is_some());
    assert!(index.get("src/main.rs").is_some());
    assert!(index.get("docs/unicode_文档.txt").is_some());

    // 4. Status Check before commit
    let status_pre = compute_status(root, &index, &store, None, &ignore).unwrap();
    assert_eq!(status_pre.staged_new.len(), 4); // README, main.rs, unicode, .spineignore
    assert!(status_pre.unstaged_modified.is_empty());
    assert!(status_pre.untracked.is_empty()); // debug.log is ignored

    // 5. Commit
    let root_tree_id = write_index_to_tree(&index, &store).unwrap();
    let commit = Commit {
        root_tree: root_tree_id,
        parents: vec![],
        author_name: "Test Developer".into(),
        author_email: "test@spine.org".into(),
        timestamp_nanos: 1730000000000000000,
        message: "feat: initial commit".into(),
    };
    let commit_bytes = spine_format::serialize_commit(&commit);
    let commit_1_id = store
        .write_object(ObjectType::Commit, &commit_bytes)
        .unwrap();

    // Update branch ref
    let ref_path = spine_dir.join("refs").join("heads").join("main");
    fs::write(&ref_path, format!("{}\n", commit_1_id.to_hex())).unwrap();

    // 6. Status check after commit
    let status_post = compute_status(root, &index, &store, Some(&root_tree_id), &ignore).unwrap();
    assert!(status_post.is_clean());

    // 7. Verify Log / History
    let history = walk_history(&store, &commit_1_id, None).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].id, commit_1_id);
    assert_eq!(history[0].commit.message, "feat: initial commit");

    // 8. Modify a file and create a second commit
    fs::write(root.join("README.md"), "# Hello Spine VCS (Modified)\n").unwrap();
    let status_mod = compute_status(root, &index, &store, Some(&root_tree_id), &ignore).unwrap();
    assert_eq!(status_mod.unstaged_modified, vec!["README.md".to_string()]);

    stage_path(root, &root.join("README.md"), &mut index, &store, &ignore).unwrap();
    index.save_to_file(spine_dir.join("index")).unwrap();

    let root_tree_2 = write_index_to_tree(&index, &store).unwrap();
    let commit_2 = Commit {
        root_tree: root_tree_2,
        parents: vec![commit_1_id],
        author_name: "Test Developer".into(),
        author_email: "test@spine.org".into(),
        timestamp_nanos: 1730000005000000000,
        message: "docs: update README".into(),
    };
    let commit_2_bytes = spine_format::serialize_commit(&commit_2);
    let commit_2_id = store
        .write_object(ObjectType::Commit, &commit_2_bytes)
        .unwrap();
    fs::write(&ref_path, format!("{}\n", commit_2_id.to_hex())).unwrap();

    let history_2 = walk_history(&store, &commit_2_id, None).unwrap();
    assert_eq!(history_2.len(), 2);
    assert_eq!(history_2[0].id, commit_2_id);
    assert_eq!(history_2[1].id, commit_1_id);

    // 9. Run FSCK on clean repository
    let fsck_clean = run_fsck(&spine_dir, &store).unwrap();
    assert!(fsck_clean.is_healthy());
    assert_eq!(fsck_clean.corrupt_objects.len(), 0);
    assert_eq!(fsck_clean.missing_objects.len(), 0);
    assert_eq!(fsck_clean.invalid_refs.len(), 0);

    // 10. Simulate corrupted object and verify FSCK detects it
    let hex = commit_2_id.to_hex();
    let (prefix, suffix) = hex.split_at(2);
    let obj_path = spine_dir.join("objects").join(prefix).join(suffix);
    let mut raw_bytes = fs::read(&obj_path).unwrap();
    let corrupt_idx = raw_bytes.len() - 1;
    raw_bytes[corrupt_idx] ^= 0xff; // corrupt byte
    fs::write(&obj_path, raw_bytes).unwrap();

    let fsck_corrupt = run_fsck(&spine_dir, &store).unwrap();
    assert!(!fsck_corrupt.is_healthy());
    assert_eq!(fsck_corrupt.corrupt_objects.len(), 1);
    assert_eq!(fsck_corrupt.corrupt_objects[0].0, commit_2_id);
}
