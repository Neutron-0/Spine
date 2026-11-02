use std::fs;
use std::process::Command;
use tempfile::tempdir;

#[test]
fn test_annotated_tags_and_verification() {
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
        .args(["config", "set", "user.name", "Release Mgr"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["config", "set", "user.email", "release@spine.org"])
        .current_dir(root)
        .status()
        .unwrap();

    // 2. Commit 1
    fs::write(root.join("version.txt"), "1.0.0\n").unwrap();
    Command::new(spine_bin)
        .args(["add", "version.txt"])
        .current_dir(root)
        .status()
        .unwrap();
    Command::new(spine_bin)
        .args(["commit", "-m", "Release v1.0.0"])
        .current_dir(root)
        .status()
        .unwrap();

    // 3. Create lightweight tag
    let out = Command::new(spine_bin)
        .args(["tag", "create", "v1.0.0-light"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Created lightweight tag"));

    // 4. Create annotated tag
    let out = Command::new(spine_bin)
        .args([
            "tag",
            "create",
            "v1.0.0",
            "-a",
            "-m",
            "Production release build v1.0.0",
        ])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).contains("Created annotated tag"));

    // 5. `spine tag list`
    let out = Command::new(spine_bin)
        .args(["tag", "list"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let list_str = String::from_utf8_lossy(&out.stdout);
    assert!(list_str.contains("v1.0.0"));
    assert!(list_str.contains("v1.0.0-light"));

    // 6. `spine tag show v1.0.0`
    let out = Command::new(spine_bin)
        .args(["tag", "show", "v1.0.0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let show_str = String::from_utf8_lossy(&out.stdout);
    assert!(show_str.contains("Tagger:     Release Mgr <release@spine.org>"));
    assert!(show_str.contains("Production release build v1.0.0"));

    // 7. `spine tag verify v1.0.0`
    let out = Command::new(spine_bin)
        .args(["tag", "verify", "v1.0.0"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    let verify_str = String::from_utf8_lossy(&out.stdout);
    assert!(verify_str.contains("Verified annotated tag 'v1.0.0'"));
    assert!(verify_str.contains("[OK]"));

    // 8. `spine tag delete v1.0.0-light`
    let out = Command::new(spine_bin)
        .args(["tag", "delete", "v1.0.0-light"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());
    assert!(String::from_utf8_lossy(&out.stdout).contains("Deleted tag 'v1.0.0-light'"));
}
