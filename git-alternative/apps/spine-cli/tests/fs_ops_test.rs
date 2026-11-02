use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_filesystem_helpers_mv_rm_clean() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Init repo and configure user
    Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit a file
    fs::write(root.join("hello.txt"), "hello world\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "hello.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Initial"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Test `spine mv hello.txt renamed.txt`
    let out = Command::new(spine_bin)
        .args(["mv", "hello.txt", "renamed.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!root.join("hello.txt").exists());
    assert!(root.join("renamed.txt").exists());

    // 4. Test `spine rm --cached renamed.txt` (keeps file on disk, removes from index)
    let out = Command::new(spine_bin)
        .args(["rm", "--cached", "renamed.txt"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(root.join("renamed.txt").exists());

    // 5. Test `spine clean`
    // First dry run
    let out = Command::new(spine_bin)
        .args(["clean", "-n"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Would remove renamed.txt"));
    assert!(root.join("renamed.txt").exists());

    // Now force clean
    let out = Command::new(spine_bin)
        .args(["clean", "-f"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(!root.join("renamed.txt").exists());
}
