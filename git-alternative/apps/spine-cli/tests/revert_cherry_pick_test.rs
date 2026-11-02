use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_revert_and_cherry_pick_workflow() {
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
        .args(["config", "set", "user.name", "Feature Dev"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "dev@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1 on main: base.txt = "base\n"
    fs::write(root.join("base.txt"), "base\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "base.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Create feature branch and add feature.txt = "feature code\n"
    Command::new(spine_bin)
        .args(["switch", "-c", "feature"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("feature.txt"), "feature code\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "feature.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Add feature"])
        .current_dir(root)
        .status()
        .unwrap();

    // Get feature commit hash
    let log_out = Command::new(spine_bin)
        .args(["log", "-l", "1"])
        .current_dir(root)
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&log_out.stdout);
    let feature_cid = log_str
        .lines()
        .next()
        .unwrap()
        .strip_prefix("commit ")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap();

    // 4. Switch back to main
    Command::new(spine_bin)
        .args(["switch", "main"])
        .current_dir(root)
        .status()
        .unwrap();
    assert!(!root.join("feature.txt").exists());

    // 5. Cherry-pick feature commit onto main
    let out = Command::new(spine_bin)
        .args(["cherry-pick", feature_cid])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(root.join("feature.txt").exists());
    assert_eq!(
        fs::read_to_string(root.join("feature.txt")).unwrap(),
        "feature code\n"
    );

    // 6. Revert the cherry-picked commit on main
    let out = Command::new(spine_bin)
        .args(["revert", "HEAD"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // After revert, feature.txt should be gone from worktree
    assert!(!root.join("feature.txt").exists());

    // Verify log has revert commit
    let log_out = Command::new(spine_bin)
        .args(["log", "-l", "1"])
        .current_dir(root)
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&log_out.stdout);
    assert!(log_str.contains("Revert"));
}
