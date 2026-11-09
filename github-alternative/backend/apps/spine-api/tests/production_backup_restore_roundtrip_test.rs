use sha2::{Digest, Sha256};
use spine_api::backup::{
    participants::ParticipantRegistry, BackupManager, BackupParticipantRecord,
    ParticipantPayloadSource, RestoreJournal, RestorePhase,
};
use spine_api::core::profile::DeploymentProfile;
use std::fs::{self, File};
use tempfile::tempdir;

/// Helper to generate a deterministic OpenSSH ed25519 key pair for test
fn generate_test_ssh_keys(ssh_dir: &std::path::Path, seed_byte: u8) -> String {
    fs::create_dir_all(ssh_dir).unwrap();
    let seed = [seed_byte; 32];
    let keypair = russh::keys::ssh_key::private::Ed25519Keypair::from_seed(&seed);
    let priv_key = russh::keys::ssh_key::PrivateKey::from(keypair);
    let pem = priv_key
        .to_openssh(russh::keys::ssh_key::LineEnding::LF)
        .unwrap();
    let pub_key = priv_key.public_key();
    let pub_str = pub_key.to_openssh().unwrap();
    let fingerprint = pub_key
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();

    fs::write(ssh_dir.join("host_key_ed25519"), pem.as_bytes()).unwrap();
    fs::write(ssh_dir.join("host_key_ed25519.pub"), pub_str.as_bytes()).unwrap();
    fingerprint
}

#[tokio::test]
async fn test_full_cluster_participant_contract_and_streaming_payload() {
    let temp = tempdir().unwrap();
    let live_storage = temp.path().join("live_storage");
    fs::create_dir_all(&live_storage).unwrap();

    // 1. Create realistic repository and configuration files in live storage
    let repo_dir = live_storage.join("repos/org/repo-alpha");
    fs::create_dir_all(&repo_dir).unwrap();
    fs::write(repo_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    fs::write(
        repo_dir.join("config"),
        "[core]\n\trepositoryformatversion = 0\n",
    )
    .unwrap();
    let pack_dir = repo_dir.join("objects/pack");
    fs::create_dir_all(&pack_dir).unwrap();
    let pack_bytes = vec![0x53, 0x50, 0x41, 0x43, 0x4b, 0x01, 0x02, 0x03, 0x04];
    fs::write(pack_dir.join("pack-alpha.spack"), &pack_bytes).unwrap();

    // 2. Setup SSH identity in storage root
    let ssh_dir = spine_api::ssh::host_key::resolve_ssh_dir(&live_storage);
    let original_fingerprint = generate_test_ssh_keys(&ssh_dir, 42);

    // 3. Capture streaming tar.gz payload for spine_storage
    let storage_snap_file = temp.path().join("spine_storage_snap.tar.gz");
    let (storage_bytes, storage_hash) =
        BackupManager::capture_storage_snapshot(&live_storage, &storage_snap_file)
            .expect("capture storage snapshot");

    assert!(storage_bytes > 0, "spine_storage payload must be non-empty");
    assert_eq!(storage_hash.len(), 64, "storage hash must be valid SHA-256");

    // 4. Capture SSH identity participant
    let fencing_generation = 42u64;
    let (ssh_record, ssh_bytes, ssh_fp) =
        BackupManager::capture_ssh_identity_snapshot(&ssh_dir, fencing_generation)
            .expect("capture ssh identity")
            .expect("ssh identity exists");
    assert_eq!(ssh_fp, original_fingerprint);

    // 5. Synthesize remaining cluster participants
    let db_snap_file = temp.path().join("db_snapshot.jsonl");
    let db_content = b"{\"record\":\"header\",\"version\":\"3.0\"}\n{\"record\":\"footer\"}\n";
    fs::write(&db_snap_file, db_content).unwrap();
    let db_hash = hex::encode(Sha256::digest(db_content));

    let pg_record = BackupParticipantRecord {
        participant_id: "postgres_data".to_string(),
        volume_name: "postgres_data".to_string(),
        snapshot_id: "pg_snap_1".to_string(),
        fencing_generation,
        byte_length: db_content.len() as u64,
        sha256_checksum: db_hash.clone(),
        archive_rel_path: "database/snapshot.jsonl".to_string(),
    };

    let storage_record = BackupParticipantRecord {
        participant_id: "spine_storage".to_string(),
        volume_name: "spine_storage".to_string(),
        snapshot_id: "storage_snap_1".to_string(),
        fencing_generation,
        byte_length: storage_bytes,
        sha256_checksum: storage_hash.clone(),
        archive_rel_path: "storage/spine_storage_snapshot.tar.gz".to_string(),
    };

    let node1_data = b"storage node 1 raw tar.gz bytes payload".to_vec();
    let node1_hash = hex::encode(Sha256::digest(&node1_data));
    let node1_record = BackupParticipantRecord {
        participant_id: "node-1".to_string(),
        volume_name: "storage_1_data".to_string(),
        snapshot_id: "node1_snap_1".to_string(),
        fencing_generation,
        byte_length: node1_data.len() as u64,
        sha256_checksum: node1_hash.clone(),
        archive_rel_path: "storage_nodes/node-1_snapshot.tar.gz".to_string(),
    };

    let node2_data = b"storage node 2 raw tar.gz bytes payload".to_vec();
    let node2_hash = hex::encode(Sha256::digest(&node2_data));
    let node2_record = BackupParticipantRecord {
        participant_id: "node-2".to_string(),
        volume_name: "storage_2_data".to_string(),
        snapshot_id: "node2_snap_1".to_string(),
        fencing_generation,
        byte_length: node2_data.len() as u64,
        sha256_checksum: node2_hash.clone(),
        archive_rel_path: "storage_nodes/node-2_snapshot.tar.gz".to_string(),
    };

    let node3_data = b"storage node 3 raw tar.gz bytes payload".to_vec();
    let node3_hash = hex::encode(Sha256::digest(&node3_data));
    let node3_record = BackupParticipantRecord {
        participant_id: "node-3".to_string(),
        volume_name: "storage_3_data".to_string(),
        snapshot_id: "node3_snap_1".to_string(),
        fencing_generation,
        byte_length: node3_data.len() as u64,
        sha256_checksum: node3_hash.clone(),
        archive_rel_path: "storage_nodes/node-3_snapshot.tar.gz".to_string(),
    };

    let coord_data = b"{\"nodes\":{},\"leases\":{}}".to_vec();
    let coord_hash = hex::encode(Sha256::digest(&coord_data));
    let coord_record = BackupParticipantRecord {
        participant_id: "coordinator".to_string(),
        volume_name: "coordinator_data".to_string(),
        snapshot_id: "coord_snap_1".to_string(),
        fencing_generation,
        byte_length: coord_data.len() as u64,
        sha256_checksum: coord_hash.clone(),
        archive_rel_path: "coordinator/coordinator_snapshot.tar.gz".to_string(),
    };

    let participants = vec![
        (
            pg_record.clone(),
            ParticipantPayloadSource::File(db_snap_file.clone()),
        ),
        (
            storage_record.clone(),
            ParticipantPayloadSource::File(storage_snap_file.clone()),
        ),
        (
            node1_record.clone(),
            ParticipantPayloadSource::Memory(node1_data.clone()),
        ),
        (
            node2_record.clone(),
            ParticipantPayloadSource::Memory(node2_data.clone()),
        ),
        (
            node3_record.clone(),
            ParticipantPayloadSource::Memory(node3_data.clone()),
        ),
        (
            coord_record.clone(),
            ParticipantPayloadSource::Memory(coord_data.clone()),
        ),
        (
            ssh_record.clone(),
            ParticipantPayloadSource::Memory(ssh_bytes.clone()),
        ),
    ];

    // Validate registry inventory
    let records: Vec<_> = participants.iter().map(|(r, _)| r.clone()).collect();
    ParticipantRegistry::validate_participants(
        &DeploymentProfile::CoreForge,
        &records,
        fencing_generation,
    )
    .expect("production participant registry validation");

    // 6. Create streaming cluster backup
    let archive_path = temp.path().join("full_cluster_backup.tar.gz");
    let manifest = BackupManager::create_cluster_backup_streaming(
        &live_storage,
        Some(&db_snap_file),
        participants,
        Some(fencing_generation),
        &archive_path,
        None,
        1,
    )
    .expect("create streaming cluster backup");

    assert_eq!(
        manifest.participants.len(),
        7,
        "All 7 authoritative participants must be in manifest"
    );
    assert_eq!(manifest.fencing_generation, Some(fencing_generation));

    // Verify all participant hashes are recorded in manifest file_hashes
    for p in &manifest.participants {
        assert_eq!(
            manifest.file_hashes.get(&p.archive_rel_path),
            Some(&p.sha256_checksum),
            "Manifest file_hashes must match participant checksum for {}",
            p.participant_id
        );
    }

    // 7. Test restore staging and verification
    let target_storage = temp.path().join("target_live_storage");
    fs::create_dir_all(&target_storage).unwrap();

    let mut session = BackupManager::prepare_restore_session(&archive_path, &target_storage, None)
        .expect("prepare restore session");

    // Check control directory
    let _control_dir = target_storage.join(RestoreJournal::CONTROL_DIR_NAME);
    let journal_file = RestoreJournal::journal_path(&target_storage);
    assert!(
        journal_file.exists(),
        "Restore journal must reside in dedicated control directory: {:?}",
        journal_file
    );

    // Verify all participant data is accessible in staging
    let fetched_node1 = session
        .get_participant_snapshot_data("storage_nodes/node-1_snapshot.tar.gz")
        .expect("fetch node1 data");
    assert_eq!(fetched_node1, node1_data);

    let fetched_coord = session
        .get_participant_snapshot_data("coordinator/coordinator_snapshot.tar.gz")
        .expect("fetch coord data");
    assert_eq!(fetched_coord, coord_data);

    // 8. Publish storage
    let report = session.publish_storage().expect("publish storage");
    assert!(report.files_restored > 0);

    // Live storage must now contain unpacked files from spine_storage snapshot
    assert!(target_storage.join("repos/org/repo-alpha/HEAD").exists());
    assert_eq!(
        fs::read_to_string(target_storage.join("repos/org/repo-alpha/HEAD")).unwrap(),
        "ref: refs/heads/main\n"
    );
    assert_eq!(
        fs::read(target_storage.join("repos/org/repo-alpha/objects/pack/pack-alpha.spack"))
            .unwrap(),
        pack_bytes
    );

    // Generation token marker must exist
    let gen = RestoreJournal::read_storage_generation(&target_storage).expect("generation marker");
    assert!(!gen.is_empty());

    // 9. Finalize
    let fin_report = session.finalize().expect("finalize restore");
    assert!(fin_report.integrity_verified);

    // Control directory and journal must be cleaned up on finalize
    assert!(
        RestoreJournal::load(&target_storage).unwrap().is_none(),
        "Journal must be cleared after finalize"
    );
}

#[tokio::test]
async fn test_participant_corruption_and_mismatch_rejection() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("source_storage");
    fs::create_dir_all(&storage_root).unwrap();
    fs::write(storage_root.join("data.txt"), b"sample payload").unwrap();

    let node1_data = b"valid node payload".to_vec();
    let node1_hash = hex::encode(Sha256::digest(&node1_data));
    let node1_record = BackupParticipantRecord {
        participant_id: "node-1".to_string(),
        volume_name: "storage_1_data".to_string(),
        snapshot_id: "node1_snap".to_string(),
        fencing_generation: 1,
        byte_length: node1_data.len() as u64,
        sha256_checksum: node1_hash,
        archive_rel_path: "storage_nodes/node-1.tar.gz".to_string(),
    };

    let archive_path = temp.path().join("archive.tar.gz");
    let participants = vec![(node1_record, ParticipantPayloadSource::Memory(node1_data))];

    BackupManager::create_cluster_backup_streaming(
        &storage_root,
        None,
        participants,
        Some(1),
        &archive_path,
        None,
        1,
    )
    .expect("create backup");

    // Tamper with archive: unpack, corrupt payload byte, repack
    let corrupt_dir = temp.path().join("corrupt_work");
    fs::create_dir_all(&corrupt_dir).unwrap();

    let f = File::open(&archive_path).unwrap();
    let dec = flate2::read::GzDecoder::new(f);
    let mut tar = tar::Archive::new(dec);
    tar.unpack(&corrupt_dir).unwrap();

    // Corrupt node-1 payload file
    let target_payload = corrupt_dir.join("storage_nodes/node-1.tar.gz");
    fs::write(&target_payload, b"CORRUPTED PAYLOAD BYTES").unwrap();

    // Repack
    let corrupt_archive = temp.path().join("corrupt_archive.tar.gz");
    let out_f = File::create(&corrupt_archive).unwrap();
    let enc = flate2::write::GzEncoder::new(out_f, flate2::Compression::default());
    let mut repacker = tar::Builder::new(enc);

    for entry in fs::read_dir(&corrupt_dir).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_file() {
            repacker.append_path_with_name(&path, &name).unwrap();
        } else if path.is_dir() {
            repacker.append_dir_all(&name, &path).unwrap();
        }
    }
    repacker.into_inner().unwrap().finish().unwrap();

    // Attempting to restore corrupt archive MUST fail closed
    let target_dir = temp.path().join("target_dir");
    fs::create_dir_all(&target_dir).unwrap();

    let restore_res = BackupManager::prepare_restore_session(&corrupt_archive, &target_dir, None);
    assert!(
        restore_res.is_err(),
        "Restoring archive with corrupted participant payload must fail"
    );
    let err_msg = restore_res.err().unwrap().to_string();
    assert!(
        err_msg.contains("Integrity verification failed")
            || err_msg.contains("checksum mismatch")
            || err_msg.contains("byte length mismatch"),
        "Error message must specify corruption: {}",
        err_msg
    );
}

#[tokio::test]
async fn test_forward_recovery_from_storage_published() {
    let temp = tempdir().unwrap();
    let storage_root = temp.path().join("live_storage");
    fs::create_dir_all(&storage_root).unwrap();

    // Simulate an interrupted restore in StoragePublished phase
    let orphan_staging = storage_root.join(".tmp_staging_test");
    fs::create_dir_all(&orphan_staging).unwrap();
    let old_gen_dir = storage_root.join(".old_generation_rst_test123");
    fs::create_dir_all(&old_gen_dir).unwrap();
    fs::write(old_gen_dir.join("old_file.txt"), b"old data").unwrap();

    let mut journal = RestoreJournal::new(
        &temp.path().join("backup.spnebak"),
        &storage_root,
        &orphan_staging,
        "test_backup",
    );
    journal.restore_id = "rst_test123".to_string();
    journal.persist(RestorePhase::StoragePublished).unwrap();

    assert!(RestoreJournal::load(&storage_root).unwrap().is_some());
    assert!(old_gen_dir.exists());

    // Execute startup recovery
    let res = RestoreJournal::handle_startup_recovery(&storage_root, None).await;
    assert!(
        res.is_ok(),
        "Startup recovery must succeed on forward recovery: {:?}",
        res.err()
    );
    let msg = res.unwrap().expect("recovery message");
    assert!(msg.contains("forward-recovered to Committed"));

    // Old generation directory must be purged
    assert!(
        !old_gen_dir.exists(),
        "Old generation must be cleaned up on forward recovery"
    );
    // Journal must be removed
    assert!(
        RestoreJournal::load(&storage_root).unwrap().is_none(),
        "Journal must be cleared"
    );
}

#[tokio::test]
async fn test_fencing_generation_monotonicity_and_fail_closed() {
    let temp = tempdir().unwrap();
    let coord_state = temp.path().join("coordinator.state");

    // Initialize new file-backed store
    let store = spine_cluster::store::FileBackedCoordinatorStore::new_file_backed(&coord_state)
        .expect("new store");

    // Persist gen 10
    store.persist_fencing_generation(10).unwrap();

    // Attempting to decrement fencing generation must fail monotonicity check
    let dec_res = store.persist_fencing_generation(5);
    assert!(
        dec_res.is_err(),
        "Decrementing fencing generation must return error"
    );
    let err = dec_res.err().unwrap().to_string();
    assert!(err.contains("Monotonicity violation"), "Error: {}", err);

    drop(store);

    // Simulate existing coordinator file with missing fencing marker
    let fencing_file = temp.path().join(".coordinator_fencing_generation");
    assert!(fencing_file.exists());
    fs::remove_file(&fencing_file).unwrap();

    // Reopening with existing state file but missing fencing marker must FAIL CLOSED
    let reopen_res =
        spine_cluster::store::FileBackedCoordinatorStore::new_file_backed(&coord_state);
    assert!(
        reopen_res.is_err(),
        "Existing coordinator participant with missing fencing marker must fail closed"
    );
    let reopen_err = reopen_res.err().unwrap().to_string();
    assert!(
        reopen_err.contains("fencing generation record") && reopen_err.contains("is missing"),
        "Error: {}",
        reopen_err
    );
}

#[tokio::test]
async fn test_ssh_identity_staged_publication_and_rollback_preservation() {
    let temp = tempdir().unwrap();
    let target_storage = temp.path().join("target_storage");
    let ssh_dir = spine_api::ssh::host_key::resolve_ssh_dir(&target_storage);
    let original_fp = generate_test_ssh_keys(&ssh_dir, 1);

    // Verify original key exists
    assert!(ssh_dir.join("host_key_ed25519").exists());

    // Generate a new test key pair in a separate staging dir
    let staging_ssh_dir = temp.path().join("staging_ssh");
    let new_fp = generate_test_ssh_keys(&staging_ssh_dir, 2);
    assert_ne!(original_fp, new_fp);

    // Simulate staged cutover with rollback preservation
    let restore_id = "test_rst_ssh_456";
    let old_ssh_dir = target_storage.join(format!(".old_ssh_{}", restore_id));

    // 1. Backup old keys to .old_ssh_<id>
    fs::create_dir_all(&old_ssh_dir).unwrap();
    for entry in fs::read_dir(&ssh_dir).unwrap().flatten() {
        spine_api::backup::safe_move_file(&entry.path(), &old_ssh_dir.join(entry.file_name()))
            .unwrap();
    }

    // 2. Publish new keys
    for entry in fs::read_dir(&staging_ssh_dir).unwrap().flatten() {
        fs::copy(entry.path(), ssh_dir.join(entry.file_name())).unwrap();
    }

    // Verify live SSH dir now has the new key
    let cur_pem = fs::read_to_string(ssh_dir.join("host_key_ed25519")).unwrap();
    let cur_key = russh::keys::ssh_key::PrivateKey::from_openssh(&cur_pem).unwrap();
    assert_eq!(
        cur_key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string(),
        new_fp
    );

    // 3. Simulate rollback: restore original keys from .old_ssh_<id>
    for entry in fs::read_dir(&old_ssh_dir).unwrap().flatten() {
        spine_api::backup::safe_move_file(&entry.path(), &ssh_dir.join(entry.file_name())).unwrap();
    }
    fs::remove_dir_all(&old_ssh_dir).unwrap();

    // Verify live SSH dir is restored to original key
    let restored_pem = fs::read_to_string(ssh_dir.join("host_key_ed25519")).unwrap();
    let restored_key = russh::keys::ssh_key::PrivateKey::from_openssh(&restored_pem).unwrap();
    assert_eq!(
        restored_key
            .public_key()
            .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
            .to_string(),
        original_fp
    );
}
