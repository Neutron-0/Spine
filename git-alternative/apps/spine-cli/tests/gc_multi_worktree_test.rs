use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_gc_protects_worktrees_and_stashes() {
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
        .args(["config", "set", "user.name", "GC Tester"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "gc@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1 on main
    fs::write(root.join("main.txt"), "main data\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "main.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Main commit"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Add linked worktree with branch 'wt-branch' and commit inside it
    let wt_dir = tempdir().unwrap();
    let wt_path = wt_dir.path().join("test_wt_gc_feature");
    let out = Command::new(spine_bin)
        .args(["worktree", "add", wt_path.to_str().unwrap(), "wt-branch"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    fs::write(wt_path.join("wt_unique.txt"), "unique worktree data\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "wt_unique.txt"])
        .current_dir(&wt_path)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Worktree unique commit"])
        .current_dir(&wt_path)
        .status()
        .unwrap();

    // 4. In main worktree, create a stash entry
    fs::write(root.join("stash_unique.txt"), "unique stash data\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "stash_unique.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["stash", "push", "-m", "Stashed unique"])
        .current_dir(root)
        .status()
        .unwrap();

    // 5. Run aggressive GC with grace-period 0s
    let out = Command::new(spine_bin)
        .args(["gc", "--prune-before", "0s"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // 6. Run fsck to prove complete repository integrity (0 broken links, 0 missing objects)
    let out = Command::new(spine_bin)
        .arg("fsck")
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let fsck_str = String::from_utf8_lossy(&out.stdout);
    assert!(fsck_str.contains("Repository is HEALTHY with zero errors"));

    // 7. Verify stash can still be applied cleanly
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
    assert!(root.join("stash_unique.txt").exists());

    // 8. Clean up linked worktree
    Command::new(spine_bin)
        .args(["worktree", "remove", wt_path.to_str().unwrap()])
        .current_dir(root)
        .status()
        .unwrap();
}
