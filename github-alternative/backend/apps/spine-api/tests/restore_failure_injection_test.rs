use sha2::Digest;
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
    let mut session =
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
    let phase = journal.unwrap().phase;
    assert!(
        phase == RestorePhase::StagedStorage || phase == RestorePhase::StagedDb,
        "Journal phase must be staged storage/db, got {:?}",
        phase
    );

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

    let mut session =
        BackupManager::prepare_restore_session(&archive_path, &live_storage, Some(passphrase))
            .expect("prepare restore session");

    // Publish storage
    let _ = session.publish_storage().expect("publish storage");
    let report = session.finalize().expect("finalize restore");
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
        "test_backup",
    );
    journal.persist(RestorePhase::StagedStorage).unwrap();

    assert!(orphan_staging.exists());
    assert!(RestoreJournal::load(&storage_root).unwrap().is_some());

    // Execute startup recovery
    let recovery_result = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
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
        "Journal should be removed after cleanup"
    );
}

#[tokio::test]
async fn test_startup_recovery_fails_closed_on_interrupted_db_cutover() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage_root");
    fs::create_dir_all(&storage_root).unwrap();

    let orphan_staging = storage_root.join(".tmp_restore_orphaned_456");
    fs::create_dir_all(&orphan_staging).unwrap();

    // Journal at DbPublished phase: target DB was updated but storage not yet published!
    let mut journal = RestoreJournal::new(
        &temp.path().join("archive.spnebak"),
        &storage_root,
        &orphan_staging,
        "test_backup_456",
    );
    journal.persist(RestorePhase::DbPublished).unwrap();

    // Startup recovery MUST FAIL CLOSED
    let recovery_result = RestoreJournal::handle_startup_recovery(&storage_root, None).await;
    assert!(
        recovery_result.is_err(),
        "Startup recovery must return an error when interrupted during cutover"
    );
    let err_msg = recovery_result.err().unwrap().to_string();
    assert!(
        err_msg.contains("FATAL STARTUP BLOCKED") || err_msg.contains("split-brain"),
        "Error message should clearly report fatal block: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_whole_generation_cutover_absent_files_do_not_survive() {
    let temp = tempdir().unwrap();
    let live_storage = temp.path().join("live_storage");
    fs::create_dir_all(&live_storage).unwrap();

    // Baseline live storage: has an active file and a stale file that should NOT exist after restore
    fs::write(live_storage.join("active.txt"), b"active data v1").unwrap();
    let stale_dir = live_storage.join("stale_dir");
    fs::create_dir_all(&stale_dir).unwrap();
    fs::write(
        stale_dir.join("should_be_deleted.txt"),
        b"ghost file from old generation",
    )
    .unwrap();

    // Backup only contains active.txt with new data
    let backup_source = temp.path().join("backup_source");
    fs::create_dir_all(&backup_source).unwrap();
    fs::write(backup_source.join("active.txt"), b"active data restored v2").unwrap();

    let archive_path = temp.path().join("clean_cutover.tar.gz");
    BackupManager::create_backup(&backup_source, None, &archive_path).expect("create backup");

    let mut session = BackupManager::prepare_restore_session(&archive_path, &live_storage, None)
        .expect("prepare session");
    let target_gen = session.journal.target_generation.clone();

    // Commit publication
    let _ = session.publish_storage().expect("publish storage");
    let report = session.finalize().expect("finalize restore");
    assert!(report.integrity_verified);

    // Verified: active.txt has restored content
    assert_eq!(
        fs::read(live_storage.join("active.txt")).unwrap(),
        b"active data restored v2"
    );

    // CRITICAL: Absent files from previous generation MUST NOT SURVIVE whole-generation replacement
    assert!(
        !live_storage.join("stale_dir").exists(),
        "Stale directory absent from backup must be deleted during whole-generation cutover"
    );
    assert!(
        !live_storage
            .join("stale_dir/should_be_deleted.txt")
            .exists(),
        "Stale file absent from backup must not survive"
    );

    // Generation token marker must be durably recorded
    let stored_gen = RestoreJournal::read_storage_generation(&live_storage).unwrap();
    assert_eq!(
        stored_gen, target_gen,
        "Live storage generation token must match target generation"
    );
}

#[tokio::test]
async fn test_multi_volume_cluster_backup_manifest_and_cutover_isolation() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage_root");
    fs::create_dir_all(&storage_root).unwrap();
    fs::write(
        storage_root.join("local_repo.txt"),
        b"spine storage content",
    )
    .unwrap();

    let archive_path = temp.path().join("cluster_backup.tar.gz");

    // Synthesize coordinator and storage node participant volumes
    let coord_data = b"dummy coordinator tar.gz snapshot payload";
    let coord_hash = hex::encode(sha2::Sha256::digest(coord_data));
    let coord_participant = spine_api::BackupParticipantRecord {
        participant_id: "coordinator".to_string(),
        volume_name: "coordinator_data".to_string(),
        snapshot_id: "coord_snap_1".to_string(),
        fencing_generation: 42,
        byte_length: coord_data.len() as u64,
        sha256_checksum: coord_hash.clone(),
        archive_rel_path: "coordinator/coordinator_snapshot.tar.gz".to_string(),
    };

    let node1_data = b"dummy storage node 1 payload";
    let node1_hash = hex::encode(sha2::Sha256::digest(node1_data));
    let node1_participant = spine_api::BackupParticipantRecord {
        participant_id: "storage-1".to_string(),
        volume_name: "storage_1_data".to_string(),
        snapshot_id: "node1_snap_1".to_string(),
        fencing_generation: 42,
        byte_length: node1_data.len() as u64,
        sha256_checksum: node1_hash.clone(),
        archive_rel_path: "storage_nodes/storage-1_snapshot.tar.gz".to_string(),
    };

    let participants = vec![
        (coord_participant, coord_data.to_vec()),
        (node1_participant, node1_data.to_vec()),
    ];

    let manifest = spine_api::backup::BackupManager::create_cluster_backup(
        &storage_root,
        None,
        participants,
        Some(42),
        &archive_path,
        None,
        1,
    )
    .expect("create cluster backup");

    assert_eq!(manifest.fencing_generation, Some(42));
    assert_eq!(manifest.participants.len(), 2);
    assert_eq!(
        manifest
            .file_hashes
            .get("coordinator/coordinator_snapshot.tar.gz"),
        Some(&coord_hash)
    );
    assert_eq!(
        manifest
            .file_hashes
            .get("storage_nodes/storage-1_snapshot.tar.gz"),
        Some(&node1_hash)
    );

    // Staging restoration session
    let target_root = temp.path().join("target_live_root");
    fs::create_dir_all(&target_root).unwrap();

    let mut session = BackupManager::prepare_restore_session(&archive_path, &target_root, None)
        .expect("prepare session");

    // Verify participant payloads are accessible in staging
    let fetched_coord = session
        .get_participant_snapshot_data("coordinator/coordinator_snapshot.tar.gz")
        .expect("fetch coordinator data");
    assert_eq!(fetched_coord, coord_data);

    let fetched_node1 = session
        .get_participant_snapshot_data("storage_nodes/storage-1_snapshot.tar.gz")
        .expect("fetch node1 data");
    assert_eq!(fetched_node1, node1_data);

    // Publish storage to live root
    let _ = session.publish_storage().expect("publish storage");
    let report = session.finalize().expect("finalize restore");
    assert!(report.integrity_verified);

    // CRITICAL: Live storage root should only receive storage files, NOT participant archives
    assert!(target_root.join("local_repo.txt").exists());
    assert!(
        !target_root.join("coordinator").exists(),
        "Participant coordinator volume must not be published into live storage root"
    );
    assert!(
        !target_root.join("storage_nodes").exists(),
        "Participant storage_nodes volumes must not be published into live storage root"
    );
}
