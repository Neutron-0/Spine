use spine_api::backup::participants::ParticipantRegistry;
use spine_api::backup::{BackupManager, BackupParticipantRecord};
use spine_api::core::profile::DeploymentProfile;
use std::fs;
use tempfile::tempdir;

#[test]
fn test_participant_registry_production_enforcement() {
    let prof = DeploymentProfile::CoreForge;
    let fencing_gen = 42u64;

    let valid_participants = vec![
        BackupParticipantRecord {
            participant_id: "node-1".to_string(),
            volume_name: "storage_1_data".to_string(),
            snapshot_id: "snap_1".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 1024,
            sha256_checksum: "a".repeat(64),
            archive_rel_path: "storage_nodes/node-1_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "node-2".to_string(),
            volume_name: "storage_2_data".to_string(),
            snapshot_id: "snap_2".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 2048,
            sha256_checksum: "b".repeat(64),
            archive_rel_path: "storage_nodes/node-2_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "node-3".to_string(),
            volume_name: "storage_3_data".to_string(),
            snapshot_id: "snap_3".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 4096,
            sha256_checksum: "c".repeat(64),
            archive_rel_path: "storage_nodes/node-3_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "coordinator".to_string(),
            volume_name: "coordinator_data".to_string(),
            snapshot_id: "snap_coord".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 512,
            sha256_checksum: "d".repeat(64),
            archive_rel_path: "coordinator/coordinator_snapshot.tar.gz".to_string(),
        },
        BackupParticipantRecord {
            participant_id: "spine_ssh".to_string(),
            volume_name: "spine_ssh".to_string(),
            snapshot_id: "snap_ssh".to_string(),
            fencing_generation: fencing_gen,
            byte_length: 768,
            sha256_checksum: "e".repeat(64),
            archive_rel_path: "ssh/ssh_identity_snapshot.tar.gz".to_string(),
        },
    ];

    // 1. Complete set passes validation
    assert!(
        ParticipantRegistry::validate_participants(&prof, &valid_participants, fencing_gen).is_ok(),
        "Complete production participant set must pass validation"
    );

    // 2. Missing any single participant fails closed under CoreForgeV1
    for i in 0..valid_participants.len() {
        let mut subset = valid_participants.clone();
        let removed = subset.remove(i);
        let err =
            ParticipantRegistry::validate_participants(&prof, &subset, fencing_gen).expect_err(
                &format!("Missing {} must fail validation", removed.participant_id),
            );
        assert!(
            err.to_string().contains("FATAL BACKUP INCOMPLETE"),
            "Error must specify FATAL BACKUP INCOMPLETE: {}",
            err
        );
        assert!(
            err.to_string().contains(&removed.participant_id),
            "Error must name missing participant {}: {}",
            removed.participant_id,
            err
        );
    }

    // 3. Fencing generation mismatch fails closed
    let mut bad_gen = valid_participants.clone();
    bad_gen[0].fencing_generation = 41;
    let err = ParticipantRegistry::validate_participants(&prof, &bad_gen, fencing_gen)
        .expect_err("Mismatched fencing generation must fail");
    assert!(err.to_string().contains("mismatched fencing generation"));

    // 4. Zero-byte participant snapshot fails closed
    let mut zero_len = valid_participants.clone();
    zero_len[1].byte_length = 0;
    let err = ParticipantRegistry::validate_participants(&prof, &zero_len, fencing_gen)
        .expect_err("Zero byte participant must fail");
    assert!(err.to_string().contains("0 bytes"));

    // 5. Invalid SHA-256 fails closed
    let mut bad_sha = valid_participants.clone();
    bad_sha[2].sha256_checksum = "corrupted".to_string();
    let err = ParticipantRegistry::validate_participants(&prof, &bad_sha, fencing_gen)
        .expect_err("Invalid sha256 checksum must fail");
    assert!(err.to_string().contains("invalid SHA-256 checksum"));

    // 6. Duplicate participant fails closed
    let mut dupes = valid_participants.clone();
    dupes.push(valid_participants[0].clone());
    let err = ParticipantRegistry::validate_participants(&prof, &dupes, fencing_gen)
        .expect_err("Duplicate participant must fail");
    assert!(err.to_string().contains("Duplicate participant ID"));

    // 7. Development profile permits partial participants
    let dev_prof = DeploymentProfile::Development;
    let single_node = vec![valid_participants[0].clone()];
    assert!(
        ParticipantRegistry::validate_participants(&dev_prof, &single_node, fencing_gen).is_ok(),
        "Development profile permits partial participants"
    );
}

#[test]
fn test_ssh_host_identity_capture_and_restore_roundtrip() {
    let temp_root = tempdir().expect("tempdir");
    let ssh_source_dir = temp_root.path().join("source_ssh");
    fs::create_dir_all(&ssh_source_dir).expect("create source_ssh");

    // 1. Generate real Ed25519 host key
    let key = spine_api::ssh::host_key::load_or_generate_host_key(temp_root.path())
        .expect("generate host key");
    let original_fp = key
        .public_key()
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();

    let ssh_dir = temp_root.path().join("ssh");
    assert!(ssh_dir.join("host_key_ed25519").exists());
    assert!(ssh_dir.join("host_key_ed25519.pub").exists());

    // 2. Capture SSH identity snapshot
    let captured = BackupManager::capture_ssh_identity_snapshot(&ssh_dir, 10)
        .expect("capture snapshot")
        .expect("snapshot must exist");

    let (record, payload, captured_fp) = captured;
    assert_eq!(record.participant_id, "spine_ssh");
    assert_eq!(record.fencing_generation, 10);
    assert_eq!(original_fp, captured_fp);
    assert!(record.byte_length > 0);
    assert_eq!(record.byte_length, payload.len() as u64);

    // 3. Restore into clean target directory
    let target_ssh_dir = temp_root.path().join("target_ssh");
    let restored_fp =
        BackupManager::restore_ssh_identity_snapshot(&target_ssh_dir, &payload, Some(&original_fp))
            .expect("restore ssh identity");

    assert_eq!(restored_fp, original_fp);
    assert!(target_ssh_dir.join("host_key_ed25519").exists());
    assert!(target_ssh_dir.join("host_key_ed25519.pub").exists());

    // 4. Key validation and verification that restored key parses and matches
    let pem = fs::read_to_string(target_ssh_dir.join("host_key_ed25519")).unwrap();
    let restored_key = russh::keys::ssh_key::PrivateKey::from_openssh(&pem).unwrap();
    assert_eq!(
        restored_key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string(),
        original_fp
    );

    // 5. Tampered fingerprint assertion fails closed
    let tampered_fp = "SHA256:0000000000000000000000000000000000000000000";
    let tamper_err = BackupManager::restore_ssh_identity_snapshot(
        &temp_root.path().join("tamper_ssh"),
        &payload,
        Some(tampered_fp),
    )
    .expect_err("Fingerprint mismatch must fail closed");
    assert!(tamper_err.to_string().contains("fingerprint mismatch"));
}

#[test]
fn test_distributed_cutover_multi_phase_rollback_preservation() {
    let temp_root = tempdir().expect("tempdir");
    let source_storage = temp_root.path().join("source_storage");
    let live_storage = temp_root.path().join("live_storage");
    fs::create_dir_all(&source_storage).unwrap();
    fs::create_dir_all(&live_storage).unwrap();

    // Populate initial live storage
    fs::write(
        live_storage.join("original.txt"),
        "original generation data",
    )
    .unwrap();

    // Populate source storage with updated content
    fs::write(
        source_storage.join("updated.txt"),
        "restored generation data",
    )
    .unwrap();

    // Create encrypted backup archive
    let archive_path = temp_root.path().join("cluster_backup.spnebak");
    let passphrase = "test-cutover-passphrase";
    let manifest = BackupManager::create_cluster_backup(
        &source_storage,
        None,
        Vec::new(),
        Some(5),
        &archive_path,
        Some(passphrase),
        2,
    )
    .expect("create backup");
    assert_eq!(manifest.fencing_generation, Some(5));

    // Stage restore session
    let mut session =
        BackupManager::prepare_restore_session(&archive_path, &live_storage, Some(passphrase))
            .expect("prepare restore session");

    // Live storage is untouched during staging
    assert!(live_storage.join("original.txt").exists());
    assert!(!live_storage.join("updated.txt").exists());

    // Phase 4 Publish: live storage updated, old generation preserved in .old_generation_<id>
    let report = session.publish_storage().expect("publish storage");
    assert!(report.integrity_verified);
    assert!(live_storage.join("updated.txt").exists());
    assert!(!live_storage.join("original.txt").exists());

    // Verify .old_generation_<id> exists
    let old_gen_dir = live_storage.join(format!(".old_generation_{}", session.journal.restore_id));
    assert!(
        old_gen_dir.exists(),
        "Old generation must be retained until finalization"
    );
    assert!(old_gen_dir.join("original.txt").exists());

    // Phase 5 Verify Failure -> Distributed Rollback
    session.rollback().expect("rollback to old generation");

    // Live storage must be restored to original state!
    assert!(
        live_storage.join("original.txt").exists(),
        "Original content must be restored after rollback"
    );
    assert_eq!(
        fs::read_to_string(live_storage.join("original.txt")).unwrap(),
        "original generation data"
    );
    assert!(
        !live_storage.join("updated.txt").exists(),
        "Updated content must be purged on rollback"
    );
    assert!(
        !old_gen_dir.exists(),
        "Old generation dir cleaned up after successful rollback"
    );
}
