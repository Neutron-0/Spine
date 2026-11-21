use std::fs;
use std::process::Command;
use tempfile::tempdir;

fn spine_bin() -> std::path::PathBuf {
    if let Ok(p) = std::env::var("CARGO_BIN_EXE_spine") {
        return std::path::PathBuf::from(p);
    }
    let mut path = std::env::current_exe().expect("failed to get current test exe");
    path.pop(); // deps
    path.pop(); // debug
    if cfg!(windows) {
        path.join("spine.exe")
    } else {
        path.join("spine")
    }
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
fn test_branch_management_and_safe_switching() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // 1. Init
    let (ok, out, _) = run_spine(root, &["init"]);
    assert!(ok, "init failed: {}", out);

    // 2. Initial commit on main
    fs::write(root.join("common.txt"), "common content\n").unwrap();
    fs::write(root.join("main_only.txt"), "main branch content\n").unwrap();
    let (ok, _, _) = run_spine(root, &["add", "."]);
    assert!(ok);
    let (ok, _, _) = run_spine(root, &["commit", "-m", "Initial commit on main"]);
    assert!(ok);

    // 3. Create branch feature-a and feature-b
    let (ok, _, _) = run_spine(root, &["branch", "create", "feature-a"]);
    assert!(ok);
    let (ok, _, _) = run_spine(root, &["branch", "create", "feature-b"]);
    assert!(ok);

    // List branches
    let (ok, out, _) = run_spine(root, &["branch"]);
    assert!(ok);
    assert!(out.contains("* main"));
    assert!(out.contains("  feature-a"));
    assert!(out.contains("  feature-b"));

    // 4. Cannot delete current branch
    let (ok, _, err) = run_spine(root, &["branch", "delete", "main"]);
    assert!(!ok);
    assert!(err.contains("Cannot delete the branch 'main' which you are currently on"));

    // 5. Switch to feature-a
    let (ok, out, _) = run_spine(root, &["switch", "feature-a"]);
    assert!(ok, "switch failed: {}", out);
    assert!(out.contains("Switched to branch 'feature-a'"));

    // Make a commit on feature-a that adds feature_file.txt and deletes main_only.txt
    fs::write(root.join("feature_file.txt"), "feature A specific\n").unwrap();
    fs::remove_file(root.join("main_only.txt")).unwrap();
    run_spine(root, &["add", "."]);
    let (ok, _, _) = run_spine(root, &["commit", "-m", "feat: work on feature A"]);
    assert!(ok);

    assert!(root.join("feature_file.txt").exists());
    assert!(!root.join("main_only.txt").exists());

    // 6. Switch back to main: feature_file.txt should disappear, main_only.txt should be restored
    let (ok, out, _) = run_spine(root, &["switch", "main"]);
    assert!(ok, "switch back to main failed: {}", out);
    assert!(
        !root.join("feature_file.txt").exists(),
        "feature_file should be removed on main"
    );
    assert!(
        root.join("main_only.txt").exists(),
        "main_only should be restored on main"
    );

    // 7. Test dirty tree protection: create an uncommitted file on a touched path
    fs::write(
        root.join("feature_file.txt"),
        "dirty untracked file that clashes\n",
    )
    .unwrap();
    let (ok, _, err) = run_spine(root, &["switch", "feature-a"]);
    assert!(!ok, "switch should have failed due to untracked collision");
    assert!(err.contains("untracked working tree files would be overwritten"));
    fs::remove_file(root.join("feature_file.txt")).unwrap();

    // 8. Test switch with uncommitted modification on UNRELATED file (common.txt was not changed between branches)
    fs::write(root.join("unrelated_dirty.txt"), "unrelated dirty work\n").unwrap();
    let (ok, out, _) = run_spine(root, &["switch", "feature-a"]);
    assert!(ok, "switch with unrelated file should succeed: {}", out);
    assert!(
        root.join("unrelated_dirty.txt").exists(),
        "unrelated dirty file must be preserved"
    );

    // 9. Rename branch feature-b to feature-c
    let (ok, _, _) = run_spine(root, &["branch", "rename", "feature-b", "feature-c"]);
    assert!(ok);
    let (ok, out, _) = run_spine(root, &["branch"]);
    assert!(ok);
    assert!(!out.contains("feature-b"));
    assert!(out.contains("feature-c"));

    // 10. Delete branch feature-c
    let (ok, out, _) = run_spine(root, &["branch", "delete", "feature-c"]);
    assert!(ok);
    assert!(out.contains("Deleted branch feature-c."));
}
