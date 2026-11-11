use flate2::write::GzEncoder;
use flate2::Compression;
use spine_api::backup::consistency::ConsistencyBarrier;
use spine_api::backup::journal::{RestoreJournal, RestorePhase};
use spine_api::backup::participants::{LocalStorageParticipantAdapter, SshParticipantAdapter};
use spine_api::backup::ParticipantAcknowledgement;
use spine_api::db::selector::{ActiveDatabaseRecord, DatabaseSelector};
use std::fs;
use tempfile::tempdir;

fn create_tar_gz_payload(entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut gz = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut gz);
        for (name, content) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            tar.append_data(&mut header, *name, *content).unwrap();
        }
        tar.finish().unwrap();
    }
    gz.finish().unwrap()
}

#[test]
fn test_genuine_local_storage_participant_adapter() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let payload = create_tar_gz_payload(&[
        ("repo1.git/HEAD", b"ref: refs/heads/main\n"),
        ("repo1.git/config", b"[core]\nrepositoryformatversion = 0\n"),
    ]);

    let restore_id = "rst_test_storage";
    let backup_id = "bak_test_storage";
    let target_gen = "105";

    // 1. Prepare staging
    let adapter =
        LocalStorageParticipantAdapter::prepare(&root, restore_id, backup_id, target_gen, &payload)
            .expect("LocalStorageParticipantAdapter::prepare should succeed");

    // 2. Validate acknowledgement binding
    let ack = &adapter.acknowledgement;
    assert_eq!(ack.restore_id, restore_id);
    assert_eq!(ack.backup_id, backup_id);
    assert_eq!(ack.participant_id, "spine_storage");
    assert_eq!(ack.canonical_resource_id, "api_local_vcs_storage");
    assert_eq!(ack.target_generation, target_gen);
    assert_eq!(ack.staged_byte_length, payload.len() as u64);
    assert!(!ack.staged_digest.is_empty());
    assert!(ack.prepared);
    assert_eq!(ack.current_state, "StagedAndVerified");

    // 3. Verify durable staging record on disk
    let query_ack = adapter
        .query_preparation_status()
        .expect("query_preparation_status should succeed")
        .expect("staging record must exist");
    assert_eq!(query_ack.staged_digest, ack.staged_digest);
    assert_eq!(query_ack.canonical_resource_id, "api_local_vcs_storage");

    // 4. Publish and verify cutover
    adapter.publish().expect("publish should succeed");
    assert!(root.join("repo1.git/HEAD").exists());
    assert_eq!(
        RestoreJournal::read_storage_generation(&root).as_deref(),
        Some(target_gen)
    );

    // 5. Finalize and verify old generation cleanup
    adapter.finalize().expect("finalize should succeed");
    assert!(!adapter.staged_dir.exists());
    assert!(!adapter.old_dir.exists());
}

#[test]
fn test_genuine_ssh_participant_adapter() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let payload = create_tar_gz_payload(&[
        (
            "host_key_ed25519",
            b"-----BEGIN OPENSSH PRIVATE KEY-----\ndummy\n-----END OPENSSH PRIVATE KEY-----\n",
        ),
        (
            "host_key_ed25519.pub",
            b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA dummy@host\n",
        ),
    ]);

    let restore_id = "rst_test_ssh";
    let backup_id = "bak_test_ssh";
    let target_gen = "106";

    // 1. Prepare staging
    let adapter =
        SshParticipantAdapter::prepare(&root, restore_id, backup_id, target_gen, &payload)
            .expect("SshParticipantAdapter::prepare should succeed");

    // 2. Validate acknowledgement binding
    let ack = &adapter.acknowledgement;
    assert_eq!(ack.restore_id, restore_id);
    assert_eq!(ack.backup_id, backup_id);
    assert_eq!(ack.participant_id, "spine_ssh");
    assert_eq!(ack.canonical_resource_id, "spine_ssh_host_identity");
    assert_eq!(ack.target_generation, target_gen);
    assert_eq!(ack.staged_byte_length, payload.len() as u64);
    assert!(!ack.staged_digest.is_empty());
    assert!(ack.prepared);
    assert_eq!(ack.current_state, "StagedAndVerified");

    // 3. Verify durable staging record on disk
    let query_ack = adapter
        .query_preparation_status()
        .expect("query_preparation_status should succeed")
        .expect("staging record must exist");
    assert_eq!(query_ack.staged_digest, ack.staged_digest);
    assert_eq!(query_ack.canonical_resource_id, "spine_ssh_host_identity");

    // 4. Publish and verify cutover
    adapter.publish().expect("publish should succeed");
    assert!(adapter.ssh_dir.join("host_key_ed25519").exists());

    // 5. Finalize and verify cleanup
    adapter.finalize().expect("finalize should succeed");
    assert!(!adapter.staged_dir.exists());
    assert!(!adapter.old_dir.exists());
}

#[test]
fn test_commit_decision_gate_strict_validation() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let restore_id = "rst_gate_test";
    let backup_id = "bak_gate_test";
    let target_gen = "200";

    let mut journal = RestoreJournal::new(
        &root.join("fake_backup.tar.gz"),
        &root,
        &tmp.path().join("staging"),
        backup_id,
    );
    journal.restore_id = restore_id.to_string();
    journal.target_generation = target_gen.to_string();
    journal.previous_storage_generation = Some("199".to_string());

    let expected = vec![
        "postgres_data".to_string(),
        "spine_storage".to_string(),
        "node-1".to_string(),
        "node-2".to_string(),
        "node-3".to_string(),
        "coordinator".to_string(),
        "spine_ssh".to_string(),
    ];

    // Gate must fail when acknowledgements are missing
    let err = journal
        .validate_commit_decision_gate(&expected)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("missing required preparation acknowledgement"));

    // Add valid acknowledgements for first 6 participants
    for id in &expected[..6] {
        let ack = ParticipantAcknowledgement {
            restore_id: restore_id.to_string(),
            backup_id: backup_id.to_string(),
            participant_id: id.clone(),
            volume_name: format!("{}_vol", id),
            canonical_resource_id: format!("canonical:{}", id),
            previous_generation: Some("199".to_string()),
            target_generation: target_gen.to_string(),
            staged_digest: "e".repeat(64),
            staged_byte_length: 1024,
            prepared_at_nanos: 1_000_000,
            prepared: true,
            durable_staging_record_id: format!("/records/{}.json", id),
            current_state: "Staged".to_string(),
        };
        journal
            .record_participant_ack(ack)
            .expect("record_participant_ack should succeed");
    }

    // Gate must still fail because 7th (spine_ssh) is missing
    let err = journal
        .validate_commit_decision_gate(&expected)
        .unwrap_err();
    assert!(err
        .to_string()
        .contains("missing required preparation acknowledgement for participant 'spine_ssh'"));

    // Record 7th with mismatched generation -> record_participant_ack must reject
    let mismatched_ack = ParticipantAcknowledgement {
        restore_id: restore_id.to_string(),
        backup_id: backup_id.to_string(),
        participant_id: "spine_ssh".to_string(),
        volume_name: "spine_ssh".to_string(),
        canonical_resource_id: "spine_ssh_host_identity".to_string(),
        previous_generation: None,
        target_generation: "999".to_string(), // Mismatched!
        staged_digest: "f".repeat(64),
        staged_byte_length: 512,
        prepared_at_nanos: 1_000_000,
        prepared: true,
        durable_staging_record_id: "/records/ssh.json".to_string(),
        current_state: "Staged".to_string(),
    };
    let rec_err = journal.record_participant_ack(mismatched_ack).unwrap_err();
    assert!(rec_err
        .to_string()
        .contains("Acknowledgement target_generation mismatch"));

    // Record valid 7th acknowledgement
    let valid_ssh_ack = ParticipantAcknowledgement {
        restore_id: restore_id.to_string(),
        backup_id: backup_id.to_string(),
        participant_id: "spine_ssh".to_string(),
        volume_name: "spine_ssh".to_string(),
        canonical_resource_id: "spine_ssh_host_identity".to_string(),
        previous_generation: None,
        target_generation: target_gen.to_string(),
        staged_digest: "f".repeat(64),
        staged_byte_length: 512,
        prepared_at_nanos: 1_000_000,
        prepared: true,
        durable_staging_record_id: "/records/ssh.json".to_string(),
        current_state: "Staged".to_string(),
    };
    journal
        .record_participant_ack(valid_ssh_ack)
        .expect("record valid ssh ack should succeed");

    // Now all 7 are present and valid -> gate must pass!
    journal
        .validate_commit_decision_gate(&expected)
        .expect("gate must succeed with all 7 genuine acks");

    // Contradictory duplicate acknowledgement with different digest must be rejected
    let contradictory_ack = ParticipantAcknowledgement {
        restore_id: restore_id.to_string(),
        backup_id: backup_id.to_string(),
        participant_id: "spine_ssh".to_string(),
        volume_name: "spine_ssh".to_string(),
        canonical_resource_id: "spine_ssh_host_identity".to_string(),
        previous_generation: None,
        target_generation: target_gen.to_string(),
        staged_digest: "0".repeat(64), // Contradictory digest!
        staged_byte_length: 512,
        prepared_at_nanos: 1_000_000,
        prepared: true,
        durable_staging_record_id: "/records/ssh.json".to_string(),
        current_state: "Staged".to_string(),
    };
    let dup_err = journal
        .record_participant_ack(contradictory_ack)
        .unwrap_err();
    assert!(dup_err
        .to_string()
        .contains("Contradictory duplicate acknowledgement"));
}

#[tokio::test]
async fn test_database_selector_filesystem_and_url_resolution() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let record = ActiveDatabaseRecord {
        base_db_name: "spine_forge".to_string(),
        active_db_name: "spine_forge_gen_42".to_string(),
        active_generation: "42".to_string(),
        restore_id: "rst_sel_test".to_string(),
        cutover_at_nanos: 1_700_000_000_000_000,
    };

    // 1. Save and load from control file
    DatabaseSelector::save_to_file(&root, &record).expect("save_to_file should succeed");
    let loaded = DatabaseSelector::load_from_file(&root)
        .expect("load_from_file should succeed")
        .expect("record must exist");
    assert_eq!(loaded.base_db_name, "spine_forge");
    assert_eq!(loaded.active_db_name, "spine_forge_gen_42");
    assert_eq!(loaded.active_generation, "42");
    assert_eq!(loaded.restore_id, "rst_sel_test");

    // 2. Resolve active DB URL using filesystem control file
    let base_url = "postgres://app_user:secret@localhost:5432/spine_forge";
    let resolved_url = DatabaseSelector::resolve_active_db_url(base_url, &root)
        .await
        .expect("resolve_active_db_url should succeed");

    assert_eq!(
        resolved_url,
        "postgres://app_user:secret@localhost:5432/spine_forge_gen_42"
    );

    // Unrelated database name is untouched
    let other_url = "postgres://app_user:secret@localhost:5432/other_db";
    let resolved_other = DatabaseSelector::resolve_active_db_url(other_url, &root)
        .await
        .expect("resolve_active_db_url should succeed");
    assert_eq!(resolved_other, other_url);
}

#[test]
fn test_restore_journal_failure_safe_durability() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let mut journal = RestoreJournal::new(
        &root.join("fake_backup.tar.gz"),
        &root,
        &tmp.path().join("staging"),
        "bak_durability",
    );
    journal.restore_id = "rst_durability".to_string();
    journal.target_generation = "50".to_string();
    journal.previous_storage_generation = Some("49".to_string());

    assert_eq!(journal.sequence_number, 1);

    // Persist phase Prepared
    journal
        .persist(RestorePhase::Prepared)
        .expect("persist Prepared should succeed");
    assert_eq!(journal.sequence_number, 2);

    // Persist phase Published
    journal
        .persist(RestorePhase::Published)
        .expect("persist Published should succeed");
    assert_eq!(journal.sequence_number, 3);

    // Verify .last_valid backup file exists
    let last_valid = RestoreJournal::last_valid_journal_path(&root);
    assert!(last_valid.exists());

    // Read back journal directly from disk
    let loaded = RestoreJournal::load(&root)
        .expect("load should succeed")
        .expect("journal must exist");
    assert_eq!(loaded.sequence_number, 3);
    assert!(matches!(loaded.phase, RestorePhase::Published));
}

#[tokio::test]
async fn test_consistency_barrier_fail_closed_checks() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let barrier = ConsistencyBarrier::new().with_storage_root(root.clone());

    // Set storage generation = "10"
    RestoreJournal::write_storage_generation(&root, "10").unwrap();

    // 1. With no active restore and aligned generation, permit is acquired
    let permit = barrier.acquire_write_permit().await;
    assert!(permit.is_ok());

    // 2. Introduce unaligned database selector generation ("20" != "10")
    let record = ActiveDatabaseRecord {
        base_db_name: "test_db".to_string(),
        active_db_name: "test_db_gen_20".to_string(),
        active_generation: "20".to_string(), // Mismatched!
        restore_id: "rst_unaligned".to_string(),
        cutover_at_nanos: 1_000_000,
    };
    DatabaseSelector::save_to_file(&root, &record).unwrap();

    let unaligned_res = barrier.acquire_write_permit().await;
    match unaligned_res {
        Err(e) => {
            let err_msg = e.to_string();
            assert!(err_msg.contains(
                "active database selector generation '20' does not match storage generation '10'"
            ));
        }
        Ok(_) => panic!("Expected unaligned generation error"),
    }

    // 3. Fix selector to match storage generation "10"
    let aligned_record = ActiveDatabaseRecord {
        active_generation: "10".to_string(),
        ..record
    };
    DatabaseSelector::save_to_file(&root, &aligned_record).unwrap();
    assert!(barrier.acquire_write_permit().await.is_ok());

    // 4. Create in-progress restore journal (Prepared phase)
    let mut journal = RestoreJournal::new(
        &root.join("fake_backup.tar.gz"),
        &root,
        &tmp.path().join("staging"),
        "bak_in_progress",
    );
    journal.restore_id = "rst_in_progress".to_string();
    journal.target_generation = "11".to_string();
    journal.previous_storage_generation = Some("10".to_string());
    journal.persist(RestorePhase::Prepared).unwrap();

    let paused_res = barrier.acquire_write_permit().await;
    match paused_res {
        Err(e) => {
            let paused_msg = e.to_string();
            assert!(paused_msg.contains("Cluster write pipeline is currently paused: restore session 'rst_in_progress' in progress"));
        }
        Ok(_) => panic!("Expected paused restore session error"),
    }
}
