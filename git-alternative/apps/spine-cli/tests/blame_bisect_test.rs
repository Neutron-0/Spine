use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_blame_and_bisect_workflow() {
    let spine_bin = env!("CARGO_BIN_EXE_spine");
    let temp = tempdir().unwrap();
    let root = temp.path();

    // 1. Init repo and configure user
    Command::new(spine_bin)
        .arg("init")
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1 by Alice
    Command::new(spine_bin)
        .args(["config", "set", "user.name", "Alice"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "alice@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("code.rs"), "line 1 (alice)\nline 2 (alice)\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "code.rs"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Alice writes line 1 and 2"])
        .current_dir(root)
        .status()
        .unwrap();

    let out_c1 = Command::new(spine_bin)
        .args(["log", "-l", "1"])
        .current_dir(root)
        .output()
        .unwrap();
    let c1_hash = String::from_utf8_lossy(&out_c1.stdout)
        .lines()
        .next()
        .unwrap()
        .strip_prefix("commit ")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();

    // 3. Commit 2 by Bob (modifies line 2, introduces bug!)
    Command::new(spine_bin)
        .args(["config", "set", "user.name", "Bob"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "bob@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(
        root.join("code.rs"),
        "line 1 (alice)\nline 2 BUG (bob)\nline 3 (bob)\n",
    )
    .unwrap();
    Command::new(spine_bin)
        .args(["add", "code.rs"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Bob introduces bug on line 2"])
        .current_dir(root)
        .status()
        .unwrap();

    let out_c2 = Command::new(spine_bin)
        .args(["log", "-l", "1"])
        .current_dir(root)
        .output()
        .unwrap();
    let c2_hash = String::from_utf8_lossy(&out_c2.stdout)
        .lines()
        .next()
        .unwrap()
        .strip_prefix("commit ")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();

    // 4. Commit 3 by Charlie
    Command::new(spine_bin)
        .args(["config", "set", "user.name", "Charlie"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "charlie@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    fs::write(root.join("notes.txt"), "some notes\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "notes.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Charlie adds notes"])
        .current_dir(root)
        .status()
        .unwrap();

    let out_c3 = Command::new(spine_bin)
        .args(["log", "-l", "1"])
        .current_dir(root)
        .output()
        .unwrap();
    let c3_hash = String::from_utf8_lossy(&out_c3.stdout)
        .lines()
        .next()
        .unwrap()
        .strip_prefix("commit ")
        .unwrap()
        .split_whitespace()
        .next()
        .unwrap()
        .to_string();

    // 5. Test `spine blame code.rs`
    let out = Command::new(spine_bin)
        .args(["blame", "code.rs"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let blame_str = String::from_utf8_lossy(&out.stdout);
    assert!(blame_str.contains("Alice"));
    assert!(blame_str.contains("Bob"));
    assert!(blame_str.contains("line 1 (alice)"));
    assert!(blame_str.contains("line 2 BUG (bob)"));
    assert!(blame_str.contains("line 3 (bob)"));

    // 6. Test `spine bisect`:
    // Start bisect with bad = c3, good = c1
    let out = Command::new(spine_bin)
        .args(["bisect", "start", &c3_hash, &c1_hash])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Midpoint candidate checked out should be c2!
    let out = Command::new(spine_bin)
        .args(["bisect", "status"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let status_str = String::from_utf8_lossy(&out.stdout);
    assert!(status_str.contains(&c2_hash));

    // Mark c2 as bad -> bisect identifies c2 as first bad commit!
    let out = Command::new(spine_bin)
        .args(["bisect", "bad"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let bad_out = String::from_utf8_lossy(&out.stdout);
    assert!(bad_out.contains(&c2_hash));
    assert!(bad_out.contains("Bob introduces bug"));

    // Reset bisect back to original HEAD (c3)
    let out = Command::new(spine_bin)
        .args(["bisect", "reset"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
}
