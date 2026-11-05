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

#[test]
fn test_encrypted_backup_creation_and_verified_restoration() {
    let source_dir = tempdir().expect("source tempdir");
    let backup_dir = tempdir().expect("backup tempdir");
    let target_dir = tempdir().expect("target tempdir");

    let file1 = source_dir.path().join("secrets/token.key");
    fs::create_dir_all(file1.parent().unwrap()).unwrap();
    fs::write(&file1, b"confidential-ssh-and-database-secrets-123456").unwrap();

    let db_snapshot = r#"{"organizations":[{"id":"org-1","name":"spine-sec"}]}"#;
    let enc_path = backup_dir.path().join("backup.spinebak");
    let passphrase = "VerySecureDeploymentPassphrase2026!";

    // 1. Create encrypted backup
    let manifest = BackupManager::create_encrypted_backup(
        source_dir.path(),
        Some(db_snapshot),
        &enc_path,
        passphrase,
        1,
    )
    .expect("create_encrypted_backup failed");

    assert_eq!(manifest.integrity_status, "VERIFIED");

    // Verify file header has SPNEBAK magic
    let raw_bytes = fs::read(&enc_path).unwrap();
    assert_eq!(&raw_bytes[0..8], b"SPNEBAK\x01");

    // 2. Reject restoration with wrong passphrase
    let wrong_res = BackupManager::restore_encrypted_backup(
        &enc_path,
        target_dir.path(),
        "WrongIncorrectPassphrase!",
    );
    assert!(
        wrong_res.is_err(),
        "Restoring with wrong passphrase must fail AEAD authentication"
    );

    // 3. Reject restoration if ciphertext is tampered
    let mut tampered_bytes = raw_bytes.clone();
    let last = tampered_bytes.len() - 5;
    tampered_bytes[last] ^= 0xAA;
    let tampered_path = backup_dir.path().join("tampered.spinebak");
    fs::write(&tampered_path, tampered_bytes).unwrap();

    let tampered_res = BackupManager::restore_encrypted_backup(
        &tampered_path,
        target_dir.path(),
        passphrase,
    );
    assert!(
        tampered_res.is_err(),
        "Restoring tampered ciphertext must fail AEAD tag validation"
    );

    // 4. Restore with correct passphrase
    let (restored_manifest, report, extracted_db) = BackupManager::restore_encrypted_backup(
        &enc_path,
        target_dir.path(),
        passphrase,
    )
    .expect("restore_encrypted_backup failed");

    assert_eq!(restored_manifest.backup_id, manifest.backup_id);
    assert!(report.integrity_verified);
    assert_eq!(extracted_db, Some(db_snapshot.to_string()));

    let restored_file = fs::read(target_dir.path().join("secrets/token.key")).unwrap();
    assert_eq!(
        restored_file,
        b"confidential-ssh-and-database-secrets-123456"
    );
}

#[test]
fn test_encrypted_backup_key_rotation() {
    let source_dir = tempdir().expect("source tempdir");
    let backup_dir = tempdir().expect("backup tempdir");
    let target_dir = tempdir().expect("target tempdir");

    fs::write(
        source_dir.path().join("payload.dat"),
        b"rotation payload test content",
    )
    .unwrap();

    let orig_enc_path = backup_dir.path().join("orig.spinebak");
    let rotated_enc_path = backup_dir.path().join("rotated.spinebak");

    let old_passphrase = "InitialPassphrase_v1!";
    let new_passphrase = "RotatedPassphrase_v2!987";

    // 1. Create with v1 passphrase
    BackupManager::create_encrypted_backup(
        source_dir.path(),
        None,
        &orig_enc_path,
        old_passphrase,
        1,
    )
    .expect("create initial backup");

    // 2. Rotate to v2 passphrase
    BackupManager::rotate_backup_key(
        &orig_enc_path,
        &rotated_enc_path,
        old_passphrase,
        new_passphrase,
        2,
    )
    .expect("rotate key");

    // 3. Old passphrase fails on rotated backup
    let old_attempt = BackupManager::restore_encrypted_backup(
        &rotated_enc_path,
        target_dir.path(),
        old_passphrase,
    );
    assert!(old_attempt.is_err(), "Old passphrase must be rejected on rotated backup");

    // 4. New passphrase succeeds
    let (_, report, _) = BackupManager::restore_encrypted_backup(
        &rotated_enc_path,
        target_dir.path(),
        new_passphrase,
    )
    .expect("restore rotated backup");

    assert!(report.integrity_verified);
    let content = fs::read(target_dir.path().join("payload.dat")).unwrap();
    assert_eq!(content, b"rotation payload test content");
}

#[test]
fn test_path_traversal_hardening() {
    let dest_root = tempdir().expect("dest tempdir");

    // Valid paths
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "valid/file.txt").is_ok());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "nested/deep/repo/HEAD").is_ok());

    // Path traversal attacks
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "../evil.txt").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "foo/../../evil.txt").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "/root/etc/passwd").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "\\Windows\\System32").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "C:\\Windows\\cmd.exe").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "CON").is_err());
    assert!(BackupManager::sanitize_restore_path(dest_root.path(), "dir/NUL").is_err());
}
