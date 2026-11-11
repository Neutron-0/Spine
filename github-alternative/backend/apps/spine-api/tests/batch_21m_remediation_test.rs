use spine_api::backup::journal::{RestoreJournal, RestorePhase};
use spine_api::backup::{safe_atomic_replace, sync_dir};
use spine_api::db::selector::{ActiveDatabaseRecord, DatabaseSelector};
use spine_cluster::protocol::ParticipantAcknowledgement;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_active_database_record_schema_and_integrity_hash() {
    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let record = ActiveDatabaseRecord::new_record(
        42,
        "rst_21m_test".to_string(),
        "decision_21m_test".to_string(),
        "a".repeat(64),
        "spine_db".to_string(),
        "gen_1".to_string(),
        "spine_db_gen_2".to_string(),
        "gen_2".to_string(),
        "Committed".to_string(),
        now_nanos,
        "spine_db".to_string(),
    );

    assert_eq!(record.selector_schema_version, 1);
    assert_eq!(record.sequence_number, 42);
    assert_eq!(record.restore_id, "rst_21m_test");
    assert_eq!(record.decision_id, "decision_21m_test");
    assert_eq!(record.backup_digest, "a".repeat(64));
    assert_eq!(record.previous_db_name, "spine_db");
    assert_eq!(record.previous_generation, "gen_1");
    assert_eq!(record.active_db_name, "spine_db_gen_2");
    assert_eq!(record.target_generation, "gen_2");
    assert_eq!(record.publication_state, "Committed");
    assert_eq!(record.base_db_name, "spine_db");
    assert_eq!(record.integrity_hash.len(), 64);
    assert!(record.verify_integrity().is_ok());

    // Tampered record must fail integrity check
    let mut tampered = record.clone();
    tampered.active_db_name = "rogue_db".to_string();
    assert!(tampered.verify_integrity().is_err());

    // Schema mismatch must fail
    let mut bad_schema = record.clone();
    bad_schema.selector_schema_version = 99;
    assert!(bad_schema.verify_integrity().is_err());
}

#[tokio::test]
async fn test_database_selector_fail_closed_resolution() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let record = ActiveDatabaseRecord::new_record(
        1,
        "rst_bootstrap".to_string(),
        "decision_bootstrap".to_string(),
        "b".repeat(64),
        "prod_app".to_string(),
        "0".to_string(),
        "prod_app_gen_1".to_string(),
        "1".to_string(),
        "Committed".to_string(),
        1_000_000,
        "prod_app".to_string(),
    );

    // Diagnostic cache round-trip
    DatabaseSelector::save_diagnostic_cache(&root, &record).expect("save cache must succeed");
    let loaded = DatabaseSelector::load_diagnostic_cache(&root)
        .expect("load cache must succeed")
        .expect("record must exist");
    assert_eq!(loaded.active_db_name, "prod_app_gen_1");
    assert_eq!(loaded.sequence_number, 1);

    // Fail closed: without PostgreSQL maintenance database online, resolution fails closed
    let raw_url = "postgres://spine_app:secret@127.0.0.1:5432/prod_app";
    let res = DatabaseSelector::resolve_active_db_url(raw_url, Some(&root)).await;
    assert!(res.is_err(), "Must fail closed instead of silent fallback");
    let err_msg = res.unwrap_err().to_string();
    assert!(
        err_msg.contains("Failed to connect to PostgreSQL maintenance database"),
        "Unexpected error: {}",
        err_msg
    );

    // Non-PostgreSQL URL passes through
    let memory_url = "sqlite::memory:";
    let pass = DatabaseSelector::resolve_active_db_url(memory_url, Some(&root)).await;
    assert_eq!(pass.unwrap(), memory_url);
}

#[test]
fn test_strict_participant_ack_exact_equality_and_digest_validation() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let mut journal = RestoreJournal::new(
        &root.join("test_backup.tar.gz"),
        &root,
        &tmp.path().join("staging"),
        "bak_target_id",
    );
    journal.restore_id = "rst_exact_42".to_string();
    journal.target_generation = "42".to_string();

    // 1. Substring restore ID (e.g. rst_exact_420 contains rst_exact_42) must be REJECTED
    let substring_restore_ack = ParticipantAcknowledgement {
        restore_id: "rst_exact_420".to_string(), // Substring match vulnerability!
        backup_id: "bak_target_id".to_string(),
        participant_id: "spine_storage".to_string(),
        volume_name: "spine_storage".to_string(),
        canonical_resource_id: "api_local_vcs_storage".to_string(),
        previous_generation: None,
        target_generation: "42".to_string(),
        staged_digest: "c".repeat(64),
        archive_payload_sha256: "c".repeat(64),
        staged_resource_digest: "c".repeat(64),
        staged_byte_length: 1024,
        prepared_at_nanos: 1_000,
        prepared: true,
        durable_staging_record_id: "/records/1".to_string(),
        current_state: "Staged".to_string(),
    };
    let err = journal
        .record_participant_ack(substring_restore_ack)
        .unwrap_err();
    assert!(err.to_string().contains("restore_id mismatch"));

    // 2. Substring backup ID must be REJECTED
    let substring_backup_ack = ParticipantAcknowledgement {
        restore_id: "rst_exact_42".to_string(),
        backup_id: "bak_target_id_extended".to_string(),
        participant_id: "spine_storage".to_string(),
        volume_name: "spine_storage".to_string(),
        canonical_resource_id: "api_local_vcs_storage".to_string(),
        previous_generation: None,
        target_generation: "42".to_string(),
        staged_digest: "c".repeat(64),
        archive_payload_sha256: "c".repeat(64),
        staged_resource_digest: "c".repeat(64),
        staged_byte_length: 1024,
        prepared_at_nanos: 1_000,
        prepared: true,
        durable_staging_record_id: "/records/1".to_string(),
        current_state: "Staged".to_string(),
    };
    let err = journal
        .record_participant_ack(substring_backup_ack)
        .unwrap_err();
    assert!(err.to_string().contains("backup_id mismatch"));

    // 3. Invalid non-hex or invalid length digest must be REJECTED
    let invalid_digest_ack = ParticipantAcknowledgement {
        restore_id: "rst_exact_42".to_string(),
        backup_id: "bak_target_id".to_string(),
        participant_id: "spine_storage".to_string(),
        volume_name: "spine_storage".to_string(),
        canonical_resource_id: "api_local_vcs_storage".to_string(),
        previous_generation: None,
        target_generation: "42".to_string(),
        staged_digest: "not_a_valid_64_hex_hash".to_string(),
        archive_payload_sha256: "not_a_valid_64_hex_hash".to_string(),
        staged_resource_digest: "not_a_valid_64_hex_hash".to_string(),
        staged_byte_length: 1024,
        prepared_at_nanos: 1_000,
        prepared: true,
        durable_staging_record_id: "/records/1".to_string(),
        current_state: "Staged".to_string(),
    };
    let err = journal
        .record_participant_ack(invalid_digest_ack)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("must be exactly 64 hex characters"));

    // 4. Exact valid acknowledgement must succeed
    let valid_ack = ParticipantAcknowledgement {
        restore_id: "rst_exact_42".to_string(),
        backup_id: "bak_target_id".to_string(),
        participant_id: "spine_storage".to_string(),
        volume_name: "spine_storage".to_string(),
        canonical_resource_id: "api_local_vcs_storage".to_string(),
        previous_generation: None,
        target_generation: "42".to_string(),
        staged_digest: "d".repeat(64),
        archive_payload_sha256: "d".repeat(64),
        staged_resource_digest: "d".repeat(64),
        staged_byte_length: 1024,
        prepared_at_nanos: 1_000,
        prepared: true,
        durable_staging_record_id: "/records/1".to_string(),
        current_state: "Staged".to_string(),
    };
    assert!(journal.record_participant_ack(valid_ack).is_ok());
}

#[test]
fn test_windows_directory_durability_and_safe_atomic_replace() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("durability_dir");
    fs::create_dir_all(&dir).unwrap();

    // Verifies sync_dir succeeds on Windows without throwing OS error 650
    assert!(sync_dir(&dir).is_ok());

    let file_a = dir.join("file_a.txt");
    let file_b = dir.join("file_b.txt");
    fs::write(&file_a, "version 1").unwrap();

    // Safe atomic replacement when destination does not yet exist
    safe_atomic_replace(&file_a, &file_b).expect("Initial replace must succeed");
    assert!(!file_a.exists());
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "version 1");

    // Safe atomic replacement when destination already exists
    let file_c = dir.join("file_c.txt");
    fs::write(&file_c, "version 2").unwrap();
    safe_atomic_replace(&file_c, &file_b).expect("Overwrite replace must succeed");
    assert!(!file_c.exists());
    assert_eq!(fs::read_to_string(&file_b).unwrap(), "version 2");
}

#[tokio::test]
async fn test_restore_journal_durability_and_readback_integrity() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let mut journal = RestoreJournal::new(
        &root.join("backup.tar.gz"),
        &root,
        &tmp.path().join("staging"),
        "bak_durability",
    );
    journal.restore_id = "rst_durability".to_string();
    journal.target_generation = "5".to_string();

    // Initial sequence_number in new is 1
    assert_eq!(journal.sequence_number, 1);

    // Persist Initialized phase increments sequence number to 2
    journal
        .persist(RestorePhase::Initialized)
        .expect("persist Initialized must succeed");
    assert_eq!(journal.sequence_number, 2);

    // .last_valid journal must exist on subsequent updates and sequence number increments to 3
    journal
        .persist(RestorePhase::Prepared)
        .expect("persist Prepared must succeed");
    assert_eq!(journal.sequence_number, 3);
    let last_valid_path = RestoreJournal::last_valid_journal_path(&root);
    assert!(last_valid_path.exists());

    let loaded = RestoreJournal::load(&root).unwrap().unwrap();
    assert_eq!(loaded.sequence_number, 3);
    assert_eq!(loaded.restore_id, "rst_durability");
    assert_eq!(loaded.phase, RestorePhase::Prepared);
}
