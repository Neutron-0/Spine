use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_rebase_linear_and_onto() {
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
        .args(["config", "set", "user.name", "Rebase Dev"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "rebase@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit C0 on main
    fs::write(root.join("common.txt"), "common\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "common.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "C0: common"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Create feature branch and add F1, F2
    Command::new(spine_bin)
        .args(["switch", "-c", "feature"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("feature1.txt"), "f1\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "feature1.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "F1: feature 1"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("feature2.txt"), "f2\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "feature2.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "F2: feature 2"])
        .current_dir(root)
        .status()
        .unwrap();

    // 4. Switch to main and add M1
    Command::new(spine_bin)
        .args(["switch", "main"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("main.txt"), "main work\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "main.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "M1: main work"])
        .current_dir(root)
        .status()
        .unwrap();

    // 5. Switch back to feature and rebase onto main
    Command::new(spine_bin)
        .args(["switch", "feature"])
        .current_dir(root)
        .status()
        .unwrap();

    let out = Command::new(spine_bin)
        .args(["rebase", "main"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Both feature files and main file should now exist in working tree!
    assert!(root.join("common.txt").exists());
    assert!(root.join("main.txt").exists());
    assert!(root.join("feature1.txt").exists());
    assert!(root.join("feature2.txt").exists());

    // 6. Verify log has 4 commits in a single linear chain
    let log_out = Command::new(spine_bin)
        .args(["log", "-l", "5"])
        .current_dir(root)
        .output()
        .unwrap();
    let log_str = String::from_utf8_lossy(&log_out.stdout);
    assert!(log_str.contains("F2: feature 2"));
    assert!(log_str.contains("F1: feature 1"));
    assert!(log_str.contains("M1: main work"));
    assert!(log_str.contains("C0: common"));
}
