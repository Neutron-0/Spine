use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_multi_worktree_cli_workflow() {
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
        .args(["config", "set", "user.name", "Worktree Dev"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "wt@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1 on main
    fs::write(root.join("shared.txt"), "shared primary\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "shared.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Initial commit on main"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Attempt to add worktree with branch 'main' (already checked out) -> must fail!
    let wt_dir = tempdir().unwrap();
    let wt_path = wt_dir.path().join("test_wt_feature");
    let out = Command::new(spine_bin)
        .args(["worktree", "add", wt_path.to_str().unwrap(), "main"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let err_str = String::from_utf8_lossy(&out.stderr);
    assert!(err_str.contains("already checked out"));

    // 4. Add worktree with branch 'feature'
    let out = Command::new(spine_bin)
        .args(["worktree", "add", wt_path.to_str().unwrap(), "feature"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Linked worktree has its own directory, .spine pointer file, and checked out files
    assert!(wt_path.join(".spine").is_file());
    assert!(wt_path.join("shared.txt").is_file());

    // 5. Work inside linked worktree: add a commit
    fs::write(wt_path.join("feature_file.txt"), "feature work\n").unwrap();
    let out = Command::new(spine_bin)
        .args(["add", "feature_file.txt"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    assert!(out.status.success());

    let out = Command::new(spine_bin)
        .args(["commit", "-m", "Feature commit in linked worktree"])
        .current_dir(&wt_path)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 6. Primary worktree should NOT have feature_file.txt in its worktree
    assert!(!root.join("feature_file.txt").exists());

    // 7. But primary worktree shares the object store: can view the commit or branch!
    let out = Command::new(spine_bin)
        .args(["show", "feature"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Feature commit in linked worktree"));

    // 8. `spine worktree list` from primary
    let out = Command::new(spine_bin)
        .args(["worktree", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let list_str = String::from_utf8_lossy(&out.stdout);
    assert!(list_str.contains("[main]"));
    assert!(list_str.contains("[feature]"));

    // 9. `spine worktree remove`
    let out = Command::new(spine_bin)
        .args(["worktree", "remove", wt_path.to_str().unwrap()])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(!wt_path.exists());
}
