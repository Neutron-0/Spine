use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn spine_bin() -> std::path::PathBuf {
    let mut path = std::env::current_exe().expect("failed to get current test exe");
    path.pop(); // deps
    path.pop(); // debug
    path.join("spine.exe")
}

fn run_spine(cwd: &std::path::Path, args: &[&str]) -> (bool, String, String) {
    let output = Command::new(spine_bin())
        .current_dir(cwd)
        .args(args)
        .output()
        .expect("failed to run spine");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (output.status.success(), stdout, stderr)
}

#[test]
fn test_undo_and_redo_workflow() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("file1.txt"), "first commit content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Commit 1"]);

    fs::write(root.join("file2.txt"), "second commit content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Commit 2"]);

    assert!(root.join("file1.txt").exists());
    assert!(root.join("file2.txt").exists());

    // Check journal list
    let (ok, out, _) = run_spine(root, &["undo", "--list"]);
    assert!(ok);
    assert!(out.contains("Transaction History"));
    assert!(out.contains("Commit: Commit 1"));
    assert!(out.contains("Commit: Commit 2"));

    // Undo Commit 2
    let (ok, out, _) = run_spine(root, &["undo"]);
    assert!(ok, "undo failed: {}", out);
    assert!(out.contains("Undid operation"));

    // file2 should no longer exist!
    assert!(root.join("file1.txt").exists());
    assert!(
        !root.join("file2.txt").exists(),
        "file2 should be removed after undoing commit 2"
    );

    // Redo Commit 2
    let (ok, out, _) = run_spine(root, &["redo"]);
    assert!(ok, "redo failed: {}", out);
    assert!(out.contains("Redid operation"));

    // file2 should be restored!
    assert!(root.join("file1.txt").exists());
    assert!(
        root.join("file2.txt").exists(),
        "file2 should be restored after redo"
    );
}

#[test]
fn test_journal_crash_recovery_on_fsck() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("file.txt"), "test\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Initial"]);

    // Simulate an interrupted transaction left in Prepared status
    let journal_records = root.join(".spine").join("journal").join("records");
    fs::create_dir_all(&journal_records).unwrap();

    let fake_record = r#"{
  "sequence": 99,
  "id": "00000000-0000-0000-0000-000000000099",
  "timestamp_nanos": 1730000000000000000,
  "kind": "Switch",
  "description": "Interrupted switch operation",
  "status": "Prepared",
  "before_state": {
    "head_ref": "refs/heads/main",
    "branch_name": "main",
    "branch_tip": null,
    "index_checksum": "none",
    "tree_id": null
  },
  "after_state": null,
  "target_tx_id": null,
  "abort_reason": null
}"#;

    fs::write(
        journal_records.join("00000099-00000000-0000-0000-0000-000000000099.json"),
        fake_record,
    )
    .unwrap();

    // Run commit to trigger journal check_and_recover
    fs::write(root.join("file.txt"), "test modified\n").unwrap();
    run_spine(root, &["add", "."]);
    let (ok, _, _) = run_spine(root, &["commit", "-m", "Trigger recovery"]);
    assert!(ok);

    // Verify the simulated record is now marked Aborted
    let content = fs::read_to_string(
        journal_records.join("00000099-00000000-0000-0000-0000-000000000099.json"),
    )
    .unwrap();
    assert!(content.contains("\"Aborted\""));
    assert!(content.contains("Interrupted by crash"));
}
