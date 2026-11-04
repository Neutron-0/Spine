use spine_api::backup::BackupManager;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_backup_creation_and_verified_restoration() {
    let source_dir = tempdir().expect("source tempdir");
    let backup_dir = tempdir().expect("backup tempdir");
    let target_dir = tempdir().expect("target tempdir");

    // 1. Populate source directory with various repositories, packfiles, artifacts
    let repo_dir = source_dir.path().join("repos/org/project.spine");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(repo_dir.join("config"), "[core]\n\tbare = true\n").unwrap();

    let objects_dir = repo_dir.join("objects/pack");
    fs::create_dir_all(&objects_dir).unwrap();
    let sample_pack = vec![0x53, 0x50, 0x41, 0x43, 0x4b, 0x01, 0x00, 0x00, 0x99, 0x88];
    fs::write(objects_dir.join("pack-test.spack"), &sample_pack).unwrap();

    let artifacts_dir = source_dir.path().join("artifacts/releases/v1");
    fs::create_dir_all(&artifacts_dir).unwrap();
    fs::write(
        artifacts_dir.join("release-v1.0.0.tar.gz"),
        b"mock release binary asset payload 1234567890",
    )
    .unwrap();

    let db_snapshot = r#"{"users":[{"id":"admin-1","username":"root"}],"repos":[{"id":"repo-1","name":"project"}]}"#;

    // 2. Create backup archive (.tar.gz)
    let backup_tar_gz = backup_dir.path().join("backup-2026-11-05.tar.gz");
    let manifest = BackupManager::create_backup(
        source_dir.path(),
        Some(db_snapshot),
        &backup_tar_gz,
    )
    .expect("create_backup failed");

    assert_eq!(manifest.integrity_status, "VERIFIED");
    assert!(manifest.total_files >= 4);
    assert!(manifest
        .file_hashes
        .contains_key("repos/org/project.spine/HEAD"));
    assert!(manifest
        .file_hashes
        .contains_key("repos/org/project.spine/objects/pack/pack-test.spack"));
    assert!(manifest.file_hashes.contains_key("database/snapshot.json"));

    // 3. Restore backup into target directory
    let (restored_manifest, report, extracted_db) = BackupManager::restore_backup(
        &backup_tar_gz,
        target_dir.path(),
    )
    .expect("restore_backup failed");

    assert_eq!(restored_manifest.backup_id, manifest.backup_id);
    assert!(report.integrity_verified);
    assert_eq!(report.files_restored, manifest.total_files);
    assert_eq!(extracted_db, Some(db_snapshot.to_string()));

    // Verify restored file contents match originals byte-for-byte
    let restored_head =
        fs::read_to_string(target_dir.path().join("repos/org/project.spine/HEAD")).unwrap();
    assert_eq!(restored_head, "ref: refs/heads/main\n");

    let restored_pack = fs::read(
        target_dir
            .path()
            .join("repos/org/project.spine/objects/pack/pack-test.spack"),
    )
    .unwrap();
    assert_eq!(restored_pack, sample_pack);

    let restored_artifact = fs::read(
        target_dir
            .path()
            .join("artifacts/releases/v1/release-v1.0.0.tar.gz"),
    )
    .unwrap();
    assert_eq!(
        restored_artifact,
        b"mock release binary asset payload 1234567890"
    );
}

#[test]
fn test_restore_tampering_and_corruption_rejection() {
    let source_dir = tempdir().expect("source tempdir");
    let backup_dir = tempdir().expect("backup tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let test_file = source_dir.path().join("data.bin");
    fs::write(&test_file, b"authentic untampered content").unwrap();

    let backup_tar_gz = backup_dir.path().join("tampered_backup.tar.gz");
    let _manifest = BackupManager::create_backup(source_dir.path(), None, &backup_tar_gz)
        .expect("create_backup failed");

    // Tamper with the archive content
    let mut archive_bytes = fs::read(&backup_tar_gz).unwrap();
    let len = archive_bytes.len();
    if len > 50 {
        archive_bytes[len - 20] ^= 0xFF;
    }
    fs::write(&backup_tar_gz, archive_bytes).unwrap();

    // Restoring corrupted archive must fail
    let res = BackupManager::restore_backup(&backup_tar_gz, target_dir.path());
    assert!(
        res.is_err(),
        "Restoration of corrupted archive must be rejected"
    );
}
