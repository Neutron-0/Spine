use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_reset_and_restore_workflow() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Init repo and configure user
    Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .status()
        .unwrap();

    Command::new(spine_bin)
        .args(["config", "set", "user.name", "Reset Tester"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "reset@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1: file1.txt = "v1", file2.txt = "v1"
    fs::write(root.join("file1.txt"), "file1 v1\n").unwrap();
    fs::write(root.join("file2.txt"), "file2 v1\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "file1.txt", "file2.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Commit 1"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Commit 2: file1.txt = "v2"
    fs::write(root.join("file1.txt"), "file1 v2\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "file1.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Commit 2"])
        .current_dir(root)
        .status()
        .unwrap();

    // 4. Commit 3: file2.txt = "v2"
    fs::write(root.join("file2.txt"), "file2 v2\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "file2.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Commit 3"])
        .current_dir(root)
        .status()
        .unwrap();

    // 5. Test `spine reset --soft HEAD~1`:
    // HEAD moves to Commit 2. Index still has file2 v2 staged. Working file2 is v2.
    let out = Command::new(spine_bin)
        .args(["reset", "--soft", "HEAD~1"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let status_out = Command::new(spine_bin)
        .arg("status")
        .current_dir(root)
        .output()
        .unwrap();
    let status_str = String::from_utf8_lossy(&status_out.stdout);
    assert!(status_str.contains("modified:   file2.txt"));

    // 6. Test `spine reset --mixed HEAD` (unstage file2.txt):
    let out = Command::new(spine_bin)
        .args(["reset", "--mixed", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    // Working directory file2 is still v2, but it's unstaged now!
    assert_eq!(
        fs::read_to_string(root.join("file2.txt")).unwrap(),
        "file2 v2\n"
    );

    // 7. Test `spine restore file2.txt`: restores working tree from index (which is v1 from Commit 2)
    let out = Command::new(spine_bin)
        .args(["restore", "file2.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        fs::read_to_string(root.join("file2.txt")).unwrap(),
        "file2 v1\n"
    );

    // 8. Test path-specific reset:
    // Modify file1 and stage it
    fs::write(root.join("file1.txt"), "file1 dirty\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "file1.txt"])
        .current_dir(root)
        .status()
        .unwrap();

    // Unstage using path reset: `spine reset HEAD file1.txt`
    let out = Command::new(spine_bin)
        .args(["reset", "HEAD", "file1.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        fs::read_to_string(root.join("file1.txt")).unwrap(),
        "file1 dirty\n"
    );

    // 9. Test `spine reset --hard HEAD~1` (moves to Commit 1, discards dirty worktree)
    let out = Command::new(spine_bin)
        .args(["reset", "--hard", "HEAD~1"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert_eq!(
        fs::read_to_string(root.join("file1.txt")).unwrap(),
        "file1 v1\n"
    );
    assert_eq!(
        fs::read_to_string(root.join("file2.txt")).unwrap(),
        "file2 v1\n"
    );

    // 10. Test restore --source <rev>
    let out = Command::new(spine_bin)
        .args(["restore", "--source", "HEAD", "file1.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
}
