use spine_api::backup::{BackupManager, RestoreJournal, RestorePhase};
use std::fs;
use tempfile::tempdir;

#[tokio::test]
async fn test_restore_database_failure_leaves_live_storage_untouched() {
    let temp = tempdir().unwrap();
    let live_storage = temp.path().join("live_storage");
    fs::create_dir_all(&live_storage).unwrap();

    // Baseline live storage files
    let original_config = "cluster_name = 'live_production_original'\n";
    fs::write(live_storage.join("config.toml"), original_config).unwrap();
    let repo_dir = live_storage.join("repos").join("prod_repo");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("spine.db"), b"original untouched live data").unwrap();

    // Create a backup source directory with different content
    let backup_source = temp.path().join("backup_source");
    fs::create_dir_all(&backup_source).unwrap();
    fs::write(
        backup_source.join("config.toml"),
        "cluster_name = 'from_backup'\n",
    )
    .unwrap();
    let bak_repo_dir = backup_source.join("repos").join("prod_repo");
    fs::create_dir_all(&bak_repo_dir).unwrap();
    fs::write(bak_repo_dir.join("spine.db"), b"new restored data").unwrap();

    // Create encrypted backup archive
    let archive_path = temp.path().join("backup.spinebak");
    let passphrase = "test_restore_fail_passphrase_456!";
    let dummy_db_snapshot = r#"{"record":"header","version":"3.0","applied_migrations":[]}
{"record":"table_start","table":"users"}
{"record":"table_end","table":"users"}
{"record":"footer","total_tables":1,"total_rows":0}
"#;

    BackupManager::create_encrypted_backup(
        &backup_source,
        Some(dummy_db_snapshot),
        &archive_path,
        passphrase,
        2,
    )
    .expect("create backup");

    // 1. Prepare restore session (Phase 1: unpacks into staging and verifies checksums)
    let session =
        BackupManager::prepare_restore_session(&archive_path, &live_storage, Some(passphrase))
            .expect("prepare restore session");

    // CRITICAL ASSERTION: While session is staged, live storage MUST be completely untouched!
    assert_eq!(
        fs::read_to_string(live_storage.join("config.toml")).unwrap(),
        original_config,
        "Live storage config.toml must be untouched during staging"
    );
    assert_eq!(
        fs::read(repo_dir.join("spine.db")).unwrap(),
        b"original untouched live data",
        "Live repository data must be untouched during staging"
    );

    // Verify journal was persisted in staging phase
    let journal = RestoreJournal::load(&live_storage).unwrap();
    assert!(
        journal.is_some(),
        "Journal must be persisted during active restore"
    );
    assert_eq!(journal.unwrap().phase, RestorePhase::StagedStorage);

    // 2. Simulate database restore failure
    let simulated_db_err = "Simulated connection reset by peer during database transaction";
    session
        .abort(simulated_db_err)
        .expect("abort restore session");

    // CRITICAL ASSERTION: After failure abort, live storage MUST REMAIN 100% UNTOUCHED
    assert_eq!(
        fs::read_to_string(live_storage.join("config.toml")).unwrap(),
        original_config,
        "Live storage config.toml must remain original after abort"
    );
    assert_eq!(
        fs::read(repo_dir.join("spine.db")).unwrap(),
        b"original untouched live data",
        "Live repository data must remain original after abort"
    );

    // Journal must be cleaned up on abort
    assert!(
        RestoreJournal::load(&live_storage).unwrap().is_none(),
        "Journal should be removed after clean abort"
    );
}

#[tokio::test]
async fn test_restore_database_success_publishes_storage_atomically() {
    let temp = tempdir().unwrap();
    let live_storage = temp.path().join("live_storage");
    fs::create_dir_all(&live_storage).unwrap();

    let backup_source = temp.path().join("backup_source");
    fs::create_dir_all(&backup_source).unwrap();
    fs::write(
        backup_source.join("config.toml"),
        "cluster_name = 'from_successful_backup'\n",
    )
    .unwrap();

    let archive_path = temp.path().join("success_backup.spinebak");
    let passphrase = "test_success_passphrase_789!";

    BackupManager::create_encrypted_backup(&backup_source, None, &archive_path, passphrase, 2)
        .expect("create backup");

    let session =
        BackupManager::prepare_restore_session(&archive_path, &live_storage, Some(passphrase))
            .expect("prepare restore session");

    // Publish storage
    let report = session.publish_storage().expect("publish storage");
    assert!(report.integrity_verified);
    assert_eq!(report.files_restored, 1);

    // Live storage is now updated
    assert_eq!(
        fs::read_to_string(live_storage.join("config.toml")).unwrap(),
        "cluster_name = 'from_successful_backup'\n"
    );

    // Journal should be removed after completion
    assert!(
        RestoreJournal::load(&live_storage).unwrap().is_none(),
        "Journal should be removed after successful publication"
    );
}

#[tokio::test]
async fn test_startup_crash_recovery_cleans_interrupted_staging() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage_root");
    fs::create_dir_all(&storage_root).unwrap();

    // Create an interrupted staging directory
    let orphan_staging = storage_root.join(".tmp_restore_orphaned_123");
    fs::create_dir_all(&orphan_staging).unwrap();
    fs::write(
        orphan_staging.join("stage_0.dat"),
        b"staged uncommitted data",
    )
    .unwrap();

    // Create an interrupted journal file in StagedStorage phase
    let mut journal = RestoreJournal::new(
        &temp.path().join("archive.spnebak"),
        &storage_root,
        &orphan_staging,
    );
    journal.persist(RestorePhase::StagedStorage).unwrap();

    assert!(orphan_staging.exists());
    assert!(RestoreJournal::load(&storage_root).unwrap().is_some());

    // Execute startup recovery
    let recovery_result = RestoreJournal::handle_startup_recovery(&storage_root).unwrap();
    assert!(
        recovery_result.is_some(),
        "Startup recovery should report cleanup message"
    );
    let msg = recovery_result.unwrap();
    assert!(msg.contains("cleanly rolled back"));

    // Staging directory must be deleted
    assert!(
        !orphan_staging.exists(),
        "Orphaned staging directory must be cleaned up"
    );

    // Journal must be removed
    assert!(
        RestoreJournal::load(&storage_root).unwrap().is_none(),
        "Interrupted journal must be removed after rollback"
    );
}
