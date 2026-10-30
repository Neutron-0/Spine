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
fn test_fast_forward_merge() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("base.txt"), "base content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Initial base commit"]);

    // Create and switch to feature-ff
    run_spine(root, &["switch", "-c", "feature-ff"]);
    fs::write(root.join("new_feature.txt"), "fast forward content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Add new feature"]);

    // Switch back to main and merge feature-ff
    run_spine(root, &["switch", "main"]);
    assert!(!root.join("new_feature.txt").exists());

    let (ok, out, _) = run_spine(root, &["merge", "feature-ff"]);
    assert!(ok, "fast-forward merge failed: {}", out);
    assert!(out.contains("Fast-forward"));
    assert!(
        root.join("new_feature.txt").exists(),
        "merged file should exist in worktree"
    );
}

#[test]
fn test_clean_3way_merge() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("base.txt"), "base content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Base commit"]);

    // Create feature branch
    run_spine(root, &["switch", "-c", "feature-branch"]);
    fs::write(root.join("feature.txt"), "feature line\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Feature commit"]);

    // Switch to main and make a divergent commit
    run_spine(root, &["switch", "main"]);
    fs::write(root.join("main.txt"), "main line\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Main divergent commit"]);

    // Merge feature-branch into main
    let (ok, out, _) = run_spine(root, &["merge", "feature-branch"]);
    assert!(ok, "clean merge failed: {}", out);
    assert!(out.contains("Merge made by the 'three-way' strategy"));
    assert!(root.join("feature.txt").exists());
    assert!(root.join("main.txt").exists());

    // Check log shows 2 parents
    let (ok, out, _) = run_spine(root, &["log", "-l", "1"]);
    assert!(ok);
    assert!(out.contains("Merge:"));
}

#[test]
fn test_conflicted_merge_abort_and_continue() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("file.txt"), "line 1\nline 2\nline 3\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Base commit"]);

    // Feature branch modifies line 2
    run_spine(root, &["switch", "-c", "branch-a"]);
    fs::write(
        root.join("file.txt"),
        "line 1\nline 2 modified by A\nline 3\n",
    )
    .unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Modify line 2 on A"]);

    // Main modifies line 2 differently
    run_spine(root, &["switch", "main"]);
    fs::write(
        root.join("file.txt"),
        "line 1\nline 2 modified by MAIN\nline 3\n",
    )
    .unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Modify line 2 on MAIN"]);

    // Merge branch-a: conflict expected!
    let (ok, out, _) = run_spine(root, &["merge", "branch-a"]);
    assert!(ok);
    assert!(out.contains("CONFLICT"));
    assert!(out.contains("Automatic merge failed; fix conflicts"));

    let content = fs::read_to_string(root.join("file.txt")).unwrap();
    assert!(content.contains("<<<<<<< OURS"));
    assert!(content.contains("line 2 modified by MAIN"));
    assert!(content.contains("======="));
    assert!(content.contains("line 2 modified by A"));
    assert!(content.contains(">>>>>>> THEIRS"));

    // Check status reports unmerged paths
    let (ok, out, _) = run_spine(root, &["status"]);
    assert!(ok);
    assert!(out.contains("unmerged paths"));

    // Test merge --abort
    let (ok, out, _) = run_spine(root, &["merge", "--abort"]);
    assert!(ok, "merge --abort failed: {}", out);
    let restored_content = fs::read_to_string(root.join("file.txt")).unwrap();
    assert_eq!(
        restored_content,
        "line 1\nline 2 modified by MAIN\nline 3\n"
    );
    assert!(!root.join(".spine").join("MERGE_STATE").exists());

    // Trigger conflict again and resolve it with merge --continue
    run_spine(root, &["merge", "branch-a"]);
    // Resolve conflict manually
    fs::write(
        root.join("file.txt"),
        "line 1\nline 2 merged manually\nline 3\n",
    )
    .unwrap();
    run_spine(root, &["add", "file.txt"]);

    let (ok, out, err) = run_spine(root, &["merge", "--continue"]);
    assert!(
        ok,
        "merge --continue failed: stdout='{}', stderr='{}'",
        out, err
    );
    assert!(out.contains("(merge-commit)"));
    assert!(!root.join(".spine").join("MERGE_STATE").exists());
}

#[test]
fn test_diff_commands() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    run_spine(root, &["init"]);
    fs::write(root.join("test.txt"), "hello world\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Init"]);

    // Unstaged modification
    fs::write(root.join("test.txt"), "hello spine vcs\n").unwrap();
    let (ok, out, _) = run_spine(root, &["diff"]);
    assert!(ok);
    assert!(out.contains("-hello world"));
    assert!(out.contains("+hello spine vcs"));

    // Staged diff
    run_spine(root, &["add", "test.txt"]);
    let (ok, out, _) = run_spine(root, &["diff", "--staged"]);
    assert!(ok);
    assert!(out.contains("-hello world"));
    assert!(out.contains("+hello spine vcs"));

    // Diff stat
    let (ok, out, _) = run_spine(root, &["diff", "--staged", "--stat"]);
    assert!(ok);
    assert!(out.contains("test.txt"));
    assert!(out.contains("1 files changed"));
}
