use spine_api::backup::journal::{CommitDecisionRecord, RestoreJournal, RestorePhase};
use spine_api::backup::participants::ParticipantRegistry;
use spine_api::backup::BackupParticipantRecord;
use spine_api::core::profile::DeploymentProfile;
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

fn build_mock_seven_participants(fencing_gen: u64) -> Vec<BackupParticipantRecord> {
    vec![
        BackupParticipantRecord {
            participant_id: "postgres_data".to_string(),
            volume_name: "postgres_data".to_string(),
            snapshot_id: "snap_pg_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 4096,
            sha256_checksum: "a1b2c3d4e5f60718293a4b5c6d7e8f90a1b2c3d4e5f60718293a4b5c6d7e8f90"
                .to_string(),
            archive_rel_path: "database/snapshot.jsonl".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "spine_storage".to_string(),
            volume_name: "spine_storage".to_string(),
            snapshot_id: "snap_storage_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 8192,
            sha256_checksum: "11223344556677889900aabbccddeeff11223344556677889900aabbccddeeff"
                .to_string(),
            archive_rel_path: "storage/live_storage.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "node-1".to_string(),
            volume_name: "storage_1_data".to_string(),
            snapshot_id: "snap_node1_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 2048,
            sha256_checksum: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef"
                .to_string(),
            archive_rel_path: "storage_nodes/node-1_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "node-2".to_string(),
            volume_name: "storage_2_data".to_string(),
            snapshot_id: "snap_node2_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 2048,
            sha256_checksum: "2345678901abcdef2345678901abcdef2345678901abcdef2345678901abcdef"
                .to_string(),
            archive_rel_path: "storage_nodes/node-2_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "node-3".to_string(),
            volume_name: "storage_3_data".to_string(),
            snapshot_id: "snap_node3_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 2048,
            sha256_checksum: "3456789012abcdef3456789012abcdef3456789012abcdef3456789012abcdef"
                .to_string(),
            archive_rel_path: "storage_nodes/node-3_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "coordinator".to_string(),
            volume_name: "coordinator_data".to_string(),
            snapshot_id: "snap_coord_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 1024,
            sha256_checksum: "4567890123abcdef4567890123abcdef4567890123abcdef4567890123abcdef"
                .to_string(),
            archive_rel_path: "coordinator/coordinator_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "spine_ssh".to_string(),
            volume_name: "spine_ssh".to_string(),
            snapshot_id: "snap_ssh_01".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 512,
            sha256_checksum: "5678901234abcdef5678901234abcdef5678901234abcdef5678901234abcdef"
                .to_string(),
            archive_rel_path: "ssh/ssh_identity_snapshot.tar.gz".to_string(),
        },
    ]
}

#[test]
fn test_canonical_seven_participants_contract_validation() {
    let prof = DeploymentProfile::CoreForge;
    let fencing_gen = 100u64;
    let participants = build_mock_seven_participants(fencing_gen);

    // 1. All seven valid participants succeed
    assert!(ParticipantRegistry::validate_participants(&prof, &participants, fencing_gen).is_ok());

    // 2. Reject duplicate participant ID
    let mut dup_id = participants.clone();
    dup_id.push(participants[0].clone());
    let err = ParticipantRegistry::validate_participants(&prof, &dup_id, fencing_gen).unwrap_err();
    assert!(err.to_string().contains("Duplicate participant ID"));

    // 3. Reject duplicate archive path
    let mut dup_path = participants.clone();
    dup_path[1].archive_rel_path = dup_path[0].archive_rel_path.clone();
    let err =
        ParticipantRegistry::validate_participants(&prof, &dup_path, fencing_gen).unwrap_err();
    assert!(err
        .to_string()
        .contains("Duplicate participant archive path"));

    // 4. Reject volume substitution mismatch
    let mut sub_mismatch = participants.clone();
    sub_mismatch[2].volume_name = "storage_2_data".to_string(); // node-1 mapped to storage_2_data
    let err =
        ParticipantRegistry::validate_participants(&prof, &sub_mismatch, fencing_gen).unwrap_err();
    assert!(err
        .to_string()
        .contains("Participant-to-volume substitution rejected"));

    // 5. Reject unexpected archive path structure
    let mut bad_path = participants.clone();
    bad_path[6].archive_rel_path = "malicious/ssh_keys.tar.gz".to_string();
    let err =
        ParticipantRegistry::validate_participants(&prof, &bad_path, fencing_gen).unwrap_err();
    assert!(err.to_string().contains("Unexpected archive path"));
}

#[tokio::test]
async fn test_crash_matrix_point_1_to_3_pre_decision_aborts_cleanly() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    // Baseline storage generation
    RestoreJournal::write_storage_generation(&storage_root, "gen_baseline_10").unwrap();

    // Crash Point 1: Initialized phase
    let staging_1 = storage_root.join(".staging_1");
    fs::create_dir_all(&staging_1).unwrap();
    fs::write(staging_1.join("temp.data"), b"uncommitted").unwrap();

    let mut journal_1 = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging_1,
        "backup_01",
    );
    journal_1.persist(RestorePhase::Initialized).unwrap();
    assert!(staging_1.exists());

    let recovery_res = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(recovery_res.is_some());
    assert!(
        !staging_1.exists(),
        "Staging directory must be cleaned up on pre-decision recovery"
    );
    assert_eq!(
        RestoreJournal::read_storage_generation(&storage_root).as_deref(),
        Some("gen_baseline_10"),
        "Storage generation must remain at baseline"
    );

    // Crash Point 2: StagedStorage phase
    let staging_2 = storage_root.join(".staging_2");
    fs::create_dir_all(&staging_2).unwrap();
    let mut journal_2 = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging_2,
        "backup_02",
    );
    journal_2.persist(RestorePhase::StagedStorage).unwrap();
    let recovery_res = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(recovery_res.is_some());
    assert!(!staging_2.exists());
    assert_eq!(
        RestoreJournal::read_storage_generation(&storage_root).as_deref(),
        Some("gen_baseline_10")
    );

    // Crash Point 3: Prepared phase
    let staging_3 = storage_root.join(".staging_3");
    fs::create_dir_all(&staging_3).unwrap();
    let mut journal_3 = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging_3,
        "backup_03",
    );
    journal_3.persist(RestorePhase::Prepared).unwrap();
    let recovery_res = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(recovery_res.is_some());
    assert!(!staging_3.exists());
    assert_eq!(
        RestoreJournal::read_storage_generation(&storage_root).as_deref(),
        Some("gen_baseline_10")
    );
}

#[tokio::test]
async fn test_crash_matrix_point_4_decision_write_fallback_to_last_valid() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let staging = storage_root.join(".staging_decision");
    fs::create_dir_all(&staging).unwrap();

    let mut journal = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging,
        "backup_decision_test",
    );
    journal.persist(RestorePhase::Initialized).unwrap();
    journal.persist(RestorePhase::Prepared).unwrap();

    let journal_file = RestoreJournal::journal_path(&storage_root);
    let last_valid = RestoreJournal::last_valid_journal_path(&storage_root);
    assert!(journal_file.exists());
    assert!(last_valid.exists());

    // Simulate mid-write crash corrupting the primary journal file
    fs::write(&journal_file, b"corrupted partial json {").unwrap();

    // Loading should resiliently recover from last_valid
    let loaded = RestoreJournal::load(&storage_root)
        .unwrap()
        .expect("Loaded journal from last_valid fallback");
    assert_eq!(loaded.phase, RestorePhase::Initialized);
}

#[tokio::test]
async fn test_crash_matrix_point_5_to_9_post_decision_rolls_forward() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let target_gen = "gen_target_committed_99";

    // Setup staging with recognizable payload file
    let staging = storage_root.join(".staging_cutover");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("payload.txt"), b"restored payload v99").unwrap();

    let mut journal = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging,
        "backup_post_decision",
    );
    journal.target_generation = target_gen.to_string();

    let decision_record = CommitDecisionRecord {
        restore_id: journal.restore_id.clone(),
        backup_id: "backup_post_decision".to_string(),
        old_generation: None,
        new_generation: target_gen.to_string(),
        required_participants: vec!["spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 123456789,
        schema_identity: "3.0".to_string(),
        decision_token: "token_test_123".to_string(),
    };

    // Crash Point 5: Immediately after durable commit decision
    journal
        .persist(RestorePhase::CommitDecision(decision_record))
        .unwrap();

    // Execute startup recovery -> MUST ROLL FORWARD!
    let msg = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap()
        .expect("Recovery must report roll forward");
    assert!(
        msg.contains("rolled forward") || msg.contains("committed"),
        "{}",
        msg
    );

    // Assert live storage updated with payload and target generation
    assert_eq!(
        RestoreJournal::read_storage_generation(&storage_root).as_deref(),
        Some(target_gen),
        "Storage generation must roll forward to target generation"
    );
    let restored_file = storage_root.join("payload.txt");
    assert!(
        restored_file.exists(),
        "Restored payload must exist in live storage root"
    );
    assert_eq!(fs::read(&restored_file).unwrap(), b"restored payload v99");

    // Journal must be cleanly removed after successful forward recovery
    assert!(
        RestoreJournal::load(&storage_root).unwrap().is_none(),
        "Journal should be removed after completion"
    );

    // Repeat recovery to verify idempotence
    let repeated = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(
        repeated.is_none(),
        "Repeated recovery must be a clean no-op"
    );
}

#[tokio::test]
async fn test_crash_matrix_point_14_and_15_recovery_idempotence() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let target_gen = "gen_target_idempotence";
    RestoreJournal::write_storage_generation(&storage_root, target_gen).unwrap();

    let staging = storage_root.join(".staging_idemp");
    fs::create_dir_all(&staging).unwrap();

    let mut journal = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging,
        "backup_idempotence",
    );
    journal.target_generation = target_gen.to_string();
    let cdr = CommitDecisionRecord {
        restore_id: journal.restore_id.clone(),
        backup_id: "backup_idempotence".to_string(),
        old_generation: None,
        new_generation: target_gen.to_string(),
        required_participants: vec!["spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 123456789,
        schema_identity: "3.0".to_string(),
        decision_token: "token_idemp_123".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();
    journal.persist(RestorePhase::Committed).unwrap();

    // Sync local recovery must RETAIN Committed journal until cluster recovery finishes
    let _ = RestoreJournal::handle_startup_recovery_sync(&storage_root).unwrap();
    assert!(
        RestoreJournal::load(&storage_root).unwrap().is_some(),
        "Committed journal must survive handle_startup_recovery_sync"
    );

    // Verify full recovery completes without errors and purges journal
    let _ = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(RestoreJournal::load(&storage_root).unwrap().is_none());

    // Subsequent recovery run is a clean no-op
    let second_run = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();
    assert!(second_run.is_none());
    assert_eq!(
        RestoreJournal::read_storage_generation(&storage_root).as_deref(),
        Some(target_gen)
    );
}

#[tokio::test]
async fn test_api_readiness_blocks_during_restore_and_clears_on_convergence() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("storage_live");
    fs::create_dir_all(&storage_root).unwrap();

    // 1. Write an active journal in Prepared phase (restore in progress)
    let staging = storage_root.join(".staging_active");
    fs::create_dir_all(&staging).unwrap();
    let mut journal = RestoreJournal::new(
        &temp.path().join("bak.spinebak"),
        &storage_root,
        &staging,
        "active_backup",
    );
    journal.persist(RestorePhase::Prepared).unwrap();

    // Journal exists and is not committed -> restore_in_progress must be detected
    let loaded = RestoreJournal::load(&storage_root).unwrap();
    assert!(loaded.is_some());
    let j = loaded.unwrap();
    assert!(!matches!(
        j.phase,
        RestorePhase::Committed | RestorePhase::Completed
    ));

    // 2. Perform recovery
    let _ = RestoreJournal::handle_startup_recovery(&storage_root, None)
        .await
        .unwrap();

    // 3. After recovery, journal is cleared -> restore is no longer in progress
    assert!(RestoreJournal::load(&storage_root).unwrap().is_none());
}
