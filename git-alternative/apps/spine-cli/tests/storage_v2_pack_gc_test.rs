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
fn test_storage_v2_repack_pack_inspect_and_composite_traversal() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    // 1. Initialize and build repository history
    let (ok, _, _) = run_spine(root, &["init"]);
    assert!(ok);

    fs::write(
        root.join("file1.txt"),
        "First file content for pack testing\n",
    )
    .unwrap();
    fs::write(
        root.join("file2.txt"),
        "Second file with some repeated lines\nline\nline\nline\n",
    )
    .unwrap();
    run_spine(root, &["add", "."]);
    let (ok, _, _) = run_spine(root, &["commit", "-m", "Commit 1: loose objects"]);
    assert!(ok);

    fs::write(root.join("file3.txt"), "Third file added in commit 2\n").unwrap();
    run_spine(root, &["add", "."]);
    let (ok, _, _) = run_spine(root, &["commit", "-m", "Commit 2: more objects"]);
    assert!(ok);

    // 2. Count objects before repack (must be loose)
    let (ok, out, _) = run_spine(root, &["count-objects"]);
    assert!(ok);
    assert!(out.contains("Storage Engine V2 Object Count Summary"));
    assert!(out.contains("Packfiles:                   0"));

    // 3. Dry-run repack
    let (ok, out, _) = run_spine(root, &["repack", "--dry-run"]);
    assert!(ok);
    assert!(out.contains("Dry-run:"));
    assert!(out.contains("loose objects would be consolidated"));

    // Verify objects are still loose after dry run
    let (ok, out, _) = run_spine(root, &["count-objects"]);
    assert!(ok);
    assert!(out.contains("Packfiles:                   0"));

    // 4. Execute real repack with verification
    let (ok, out, err) = run_spine(root, &["repack", "--verify"]);
    assert!(ok, "repack failed: stdout='{}', stderr='{}'", out, err);
    assert!(out.contains("Repack complete:"));
    assert!(out.contains("Integrity check PASSED"));

    // 5. Count objects post-repack (must be 0 loose, 1 packfile)
    let (ok, out, err) = run_spine(root, &["count-objects"]);
    assert!(
        ok,
        "count-objects failed: stdout='{}', stderr='{}'",
        out, err
    );
    assert!(
        out.contains("Packfiles:                   1"),
        "out was: {}",
        out
    );
    assert!(
        out.contains("Loose objects:") && out.contains("0 (0 B)"),
        "out was: {}",
        out
    );

    // 6. Inspect pack list in catalog
    let (ok, out, _) = run_spine(root, &["pack", "list"]);
    assert!(ok);
    assert!(out.contains("Catalog Generation: 1"));
    assert!(out.contains("pack-"));

    // Extract pack ID
    let pack_id = out
        .lines()
        .find(|l| l.contains("pack-"))
        .and_then(|l| l.split_whitespace().next())
        .expect("Pack ID must be present in pack list");

    // 7. Inspect packfile
    let (ok, out, err) = run_spine(root, &["pack", "inspect", pack_id]);
    assert!(
        ok,
        "pack inspect failed: stdout='{}', stderr='{}'",
        out, err
    );
    assert!(out.contains(&format!("Packfile: {}", pack_id)));
    assert!(out.contains("Blobs:"));
    assert!(out.contains("Trees:"));
    assert!(out.contains("Commits:"));
    assert!(out.contains("Compression Savings:"));

    // 8. Test that log and show seamlessly traverse history from packfile
    let (ok, out, _) = run_spine(root, &["log"]);
    assert!(ok);
    assert!(out.contains("Commit 1: loose objects"));
    assert!(out.contains("Commit 2: more objects"));

    let (ok, out, _) = run_spine(root, &["show", "HEAD"]);
    assert!(ok);
    assert!(out.contains("Commit 2: more objects"));

    // 9. Verify storage integrity command
    let (ok, out, _) = run_spine(root, &["verify-storage"]);
    assert!(ok);
    assert!(out.contains("Packs audited:          1"));
    assert!(out.contains("All storage objects and packs are cryptographically healthy"));

    // 10. Verify fsck reports healthy
    let (ok, out, _) = run_spine(root, &["fsck"]);
    assert!(ok);
    assert!(out.contains("Repository is HEALTHY with zero errors"));
}

#[test]
fn test_garbage_collection_lifecycle_and_grace_period() {
    let dir = tempdir().unwrap();
    let root = dir.path();

    let (ok, _, _) = run_spine(root, &["init"]);
    assert!(ok);

    fs::write(root.join("main.txt"), "main branch content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Main commit"]);

    // Create orphan branch and commit
    run_spine(root, &["switch", "-c", "temp-orphan"]);
    fs::write(root.join("orphan.txt"), "dead orphan content\n").unwrap();
    run_spine(root, &["add", "."]);
    run_spine(root, &["commit", "-m", "Orphan commit to be deleted"]);

    // Switch back to main and delete orphan branch
    run_spine(root, &["switch", "main"]);
    let (ok, _, _) = run_spine(root, &["branch", "delete", "temp-orphan", "-f"]);
    assert!(ok);

    // 1. Standard GC with default 14-day grace period should preserve unreachable objects
    let (ok, out, _) = run_spine(root, &["gc", "--prune-before", "14d"]);
    assert!(ok);
    assert!(out.contains("Garbage collection complete"));
    assert!(out.contains("Grace-period preserved:"));
    assert!(out.contains("Pruned objects:         0"));

    // 2. Dry run with 0s grace period
    let (ok, out, _) = run_spine(root, &["gc", "--dry-run", "--prune-before", "0s"]);
    assert!(ok);
    assert!(out.contains("Dry-run:"));
    assert!(out.contains("unreachable objects eligible for pruning"));

    // 3. Real GC with 0s grace period prunes expired unreachable objects
    let (ok, out, _) = run_spine(root, &["gc", "--prune-before", "0s"]);
    assert!(ok);
    assert!(out.contains("Garbage collection complete:"));

    // 4. Storage remains healthy after GC
    let (ok, out, _) = run_spine(root, &["verify-storage"]);
    assert!(ok);
    assert!(out.contains("All storage objects and packs are cryptographically healthy"));
}
