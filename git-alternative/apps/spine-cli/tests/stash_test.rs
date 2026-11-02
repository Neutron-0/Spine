use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_stash_cli_workflow() {
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
        .args(["config", "set", "user.name", "Stash Tester"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "stash@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1: file.txt
    fs::write(root.join("file.txt"), "line 1\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "file.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Make working tree dirty (tracked modification + untracked file)
    fs::write(root.join("file.txt"), "line 1\nline 2 dirty\n").unwrap();
    fs::write(root.join("new_file.txt"), "untracked content\n").unwrap();

    // 4. `spine stash push -u -m "WIP changes"`
    let out = Command::new(spine_bin)
        .args(["stash", "push", "-u", "-m", "WIP changes"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Working directory must be clean now
    assert_eq!(
        fs::read_to_string(root.join("file.txt")).unwrap(),
        "line 1\n"
    );
    assert!(!root.join("new_file.txt").exists());

    // 5. `spine stash list`
    let out = Command::new(spine_bin)
        .args(["stash", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let list_str = String::from_utf8_lossy(&out.stdout);
    assert!(list_str.contains("stash@{0}: WIP changes"));

    // 6. `spine stash show`
    let out = Command::new(spine_bin)
        .args(["stash", "show"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let show_str = String::from_utf8_lossy(&out.stdout);
    assert!(show_str.contains("file.txt"));

    // 7. `spine stash pop`
    let out = Command::new(spine_bin)
        .args(["stash", "pop"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Dirty state restored!
    assert_eq!(
        fs::read_to_string(root.join("file.txt")).unwrap(),
        "line 1\nline 2 dirty\n"
    );
    assert!(root.join("new_file.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("new_file.txt")).unwrap(),
        "untracked content\n"
    );

    // Stash list should now be empty
    let out = Command::new(spine_bin)
        .args(["stash", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).trim().is_empty());
}
