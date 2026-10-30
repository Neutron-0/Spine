use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_cli_end_to_end_binary_workflow() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. `spine init`
    let output = Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .output()
        .expect("Failed to execute spine init");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Initialized empty Spine repository"));
    assert!(root.join(".spine").is_dir());
    assert!(root.join(".spine/spine.toml").is_file());

    // 2. Re-init in same directory must fail with error
    let output_reinit = Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .output()
        .expect("Failed to execute spine init reinit");
    assert!(!output_reinit.status.success());
    let stderr = String::from_utf8_lossy(&output_reinit.stderr);
    assert!(stderr.contains("already contains a .spine repository"));

    // 3. `spine config set`
    let output = Command::new(spine_bin)
        .args(["config", "set", "user.name", "Test Developer"])
        .current_dir(root)
        .output()
        .expect("Failed to execute config set name");
    assert!(output.status.success());

    let output = Command::new(spine_bin)
        .args(["config", "set", "user.email", "developer@spine.org"])
        .current_dir(root)
        .output()
        .expect("Failed to execute config set email");
    assert!(output.status.success());

    // Verify config get
    let output = Command::new(spine_bin)
        .args(["config", "get", "user.name"])
        .current_dir(root)
        .output()
        .expect("Failed to execute config get name");
    assert!(output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout).trim(),
        "Test Developer"
    );

    // 4. Create files (flat & nested)
    fs::write(
        root.join("README.md"),
        "# Spine Project\nHigh performance VCS.\n",
    )
    .unwrap();
    let sub = root.join("src").join("core");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("engine.rs"), "pub fn run() {}\n").unwrap();

    // 5. `spine status` before staging (must show untracked)
    let output = Command::new(spine_bin)
        .arg("status")
        .current_dir(root)
        .output()
        .expect("Failed to run status");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Untracked files:"));
    assert!(stdout.contains("README.md"));
    assert!(stdout.contains("src/core/engine.rs"));

    // 6. `spine add .`
    let output = Command::new(spine_bin)
        .args(["add", "."])
        .current_dir(root)
        .output()
        .expect("Failed to run add");
    assert!(output.status.success());

    // 7. `spine status` after staging (must show changes to be committed)
    let output = Command::new(spine_bin)
        .arg("status")
        .current_dir(root)
        .output()
        .expect("Failed to run status");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Changes to be committed:"));
    assert!(stdout.contains("new file:   README.md"));
    assert!(stdout.contains("new file:   src/core/engine.rs"));

    // 8. `spine commit -m "Initial commit"`
    let output = Command::new(spine_bin)
        .args(["commit", "-m", "Initial commit"])
        .current_dir(root)
        .output()
        .expect("Failed to run commit");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[main (root-commit)"));
    assert!(stdout.contains("Initial commit"));

    // 9. `spine log`
    let output = Command::new(spine_bin)
        .arg("log")
        .current_dir(root)
        .output()
        .expect("Failed to run log");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Author: Test Developer <developer@spine.org>"));
    assert!(stdout.contains("Initial commit"));
    assert!(stdout.contains("(HEAD -> main)"));

    // 10. `spine show HEAD`
    let output = Command::new(spine_bin)
        .args(["show", "HEAD"])
        .current_dir(root)
        .output()
        .expect("Failed to run show HEAD");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Tree contents (2 files):"));
    assert!(stdout.contains("README.md"));
    assert!(stdout.contains("src/core/engine.rs"));

    // 11. `spine fsck` on pristine repo
    let output = Command::new(spine_bin)
        .arg("fsck")
        .current_dir(root)
        .output()
        .expect("Failed to run fsck");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Repository is HEALTHY with zero errors"));

    // 12. Modify file, stage, commit second change
    fs::write(
        root.join("README.md"),
        "# Spine Project\nUpdated documentation.\n",
    )
    .unwrap();
    let output = Command::new(spine_bin)
        .args(["add", "README.md"])
        .current_dir(root)
        .output()
        .expect("Failed to add modified file");
    assert!(output.status.success());

    let output = Command::new(spine_bin)
        .args(["commit", "-m", "docs: update readme"])
        .current_dir(root)
        .output()
        .expect("Failed to commit second change");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("[main "));
    assert!(stdout.contains("docs: update readme"));

    // 13. Verify log now has 2 commits
    let output = Command::new(spine_bin)
        .arg("log")
        .current_dir(root)
        .output()
        .expect("Failed to run log second time");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("docs: update readme"));
    assert!(stdout.contains("Initial commit"));

    // 14. Final fsck
    let output = Command::new(spine_bin)
        .arg("fsck")
        .current_dir(root)
        .output()
        .expect("Failed to run final fsck");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Repository is HEALTHY with zero errors"));
}
