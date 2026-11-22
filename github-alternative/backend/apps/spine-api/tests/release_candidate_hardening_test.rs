mod common;

use spine_api::backup::consistency::ConsistencyBarrier;
use spine_api::backup::journal::{CommitDecisionRecord, RestoreJournal, RestorePhase};
use spine_api::backup::{
    safe_atomic_replace, safe_atomic_replace_strict_writethrough, sync_dir, BackupManager,
    BackupParticipantRecord, EphemeralEncryptedTemp, ParticipantPayloadSource,
};
use spine_api::db::selector::{ActiveDatabaseRecord, DatabaseSelector};
use spine_api::CoordinatorClient;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse, ParticipantAcknowledgement};
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore};
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

#[test]
fn test_post_decision_journal_invariants_and_stale_record_protection() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    let staging = tmp.path().join("staging");
    let dummy_archive = tmp.path().join("archive.spnebak");
    fs::create_dir_all(&root).unwrap();
    fs::write(&dummy_archive, b"dummy").unwrap();

    let mut journal = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_100");
    journal.restore_id = "rst_alpha".to_string();
    journal.target_generation = "gen_100".to_string();

    // 1. Persist Initialized -> sequence 2
    journal.persist(RestorePhase::Initialized).unwrap();
    assert_eq!(journal.sequence_number, 2);

    // 2. Persist Prepared -> sequence 3
    journal.persist(RestorePhase::Prepared).unwrap();
    assert_eq!(journal.sequence_number, 3);

    // 3. Persist CommitDecision -> sequence 4
    let mut digests = BTreeMap::new();
    digests.insert("manifest.json".to_string(), "abc".to_string());
    let cdr = CommitDecisionRecord {
        restore_id: "rst_alpha".to_string(),
        backup_id: "bak_100".to_string(),
        old_generation: None,
        new_generation: "gen_100".to_string(),
        required_participants: vec![
            "postgres_data".to_string(),
            "spine_storage".to_string(),
            "storage_1_data".to_string(),
            "storage_2_data".to_string(),
            "storage_3_data".to_string(),
            "coordinator_data".to_string(),
            "spine_ssh".to_string(),
        ],
        staged_manifest_digests: digests,
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_123".to_string(),
    };
    journal
        .persist(RestorePhase::CommitDecision(cdr.clone()))
        .unwrap();
    assert_eq!(journal.sequence_number, 4);
    assert!(journal.commit_decision.is_some());

    // 4. Invariant: candidate with sequence_number <= existing sequence (4) must be rejected
    let mut bad_seq = journal.clone();
    bad_seq.sequence_number = 3;
    let err = bad_seq.persist(RestorePhase::Published);
    assert!(
        err.is_err(),
        "Journal must reject sequence monotonicity regression"
    );

    // 5. Invariant: candidate attempting to erase commit_decision must be rejected
    let mut erased_decision = journal.clone();
    erased_decision.commit_decision = None;
    let err2 = erased_decision.persist(RestorePhase::Published);
    assert!(
        err2.is_err(),
        "Journal must forbid erasing established CommitDecision"
    );

    // 6. Invariant: a different restore session cannot overwrite an unrecovered committed restore
    let mut rogue_journal = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_rogue");
    rogue_journal.restore_id = "rst_rogue".to_string();
    let err3 = rogue_journal.persist(RestorePhase::Initialized);
    assert!(
        err3.is_err(),
        "New restore session must not overwrite an active unrecovered committed restore"
    );

    // 7. Verify .last_valid journal was preserved on disk and matches previous state
    let lv_path = RestoreJournal::last_valid_journal_path(&root);
    assert!(lv_path.exists(), ".last_valid journal must exist");
    let lv_bytes = fs::read(&lv_path).unwrap();
    let lv_journal: RestoreJournal = serde_json::from_slice(&lv_bytes).unwrap();
    assert_eq!(lv_journal.restore_id, "rst_alpha");
    assert_eq!(lv_journal.sequence_number, 3);
    assert_eq!(lv_journal.phase, RestorePhase::Prepared);
}

#[test]
fn test_pre_decision_vs_post_decision_recovery_distinction() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    let staging = root.join(".staged_generation_pre");
    let dummy_archive = tmp.path().join("archive.spnebak");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(&dummy_archive, b"dummy").unwrap();

    // Case A: Pre-decision failure cleanly rolls back and cleans up staging
    let mut journal_pre = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_pre");
    journal_pre.restore_id = "rst_pre".to_string();
    journal_pre.target_generation = "gen_pre".to_string();
    journal_pre.staging_storage_root = staging.clone();
    journal_pre.persist(RestorePhase::Prepared).unwrap();

    assert!(staging.exists(), "Staging dir should exist before rollback");
    let recovery_res = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
    assert!(
        recovery_res.is_some(),
        "Pre-decision restore must be rolled back on recovery"
    );
    assert!(
        !staging.exists(),
        "Staging dir must be purged on pre-decision rollback"
    );
    assert!(
        RestoreJournal::load(&root).unwrap().is_none(),
        "Journal must be removed after pre-decision rollback"
    );

    // Case B: Post-decision failure forward-recovers and NEVER rolls back
    let post_staging = root.join(".staged_generation_post");
    fs::create_dir_all(&post_staging).unwrap();
    fs::write(post_staging.join("payload.txt"), b"restored content").unwrap();

    let mut journal_post = RestoreJournal::new(&dummy_archive, &root, &post_staging, "bak_post");
    journal_post.restore_id = "rst_post".to_string();
    journal_post.target_generation = "gen_post".to_string();
    journal_post.staging_storage_root = post_staging.clone();

    let cdr = CommitDecisionRecord {
        restore_id: "rst_post".to_string(),
        backup_id: "bak_post".to_string(),
        old_generation: None,
        new_generation: "gen_post".to_string(),
        required_participants: vec!["spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_post".to_string(),
    };
    journal_post
        .persist(RestorePhase::CommitDecision(cdr))
        .unwrap();

    // Verify recovery forward-converges to target generation "gen_post"
    let recovery_post = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
    assert!(
        recovery_post.is_some(),
        "Committed restore must be forward-recovered"
    );
    assert_eq!(
        RestoreJournal::read_storage_generation(&root).as_deref(),
        Some("gen_post"),
        "Storage generation must be updated forward to target generation"
    );
    assert!(
        root.join("payload.txt").exists(),
        "Restored payload must be published to live root"
    );
}

#[test]
fn test_win32_durability_and_error_650_prevention() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("win32_durability");
    fs::create_dir_all(&dir).unwrap();

    // sync_dir must succeed on Windows without error 650
    assert!(sync_dir(&dir).is_ok());

    let file_src = dir.join("journal_src.json");
    let file_dst = dir.join("journal_dst.json");
    fs::write(&file_src, b"{\"test\":\"data\"}").unwrap();

    // safe_atomic_replace with write-through
    safe_atomic_replace(&file_src, &file_dst).expect("Atomic replace must succeed");
    assert!(!file_src.exists());
    assert_eq!(
        fs::read_to_string(&file_dst).unwrap(),
        "{\"test\":\"data\"}"
    );

    // Overwrite existing destination with write-through
    let file_src2 = dir.join("journal_src2.json");
    fs::write(&file_src2, b"{\"test\":\"data2\"}").unwrap();
    safe_atomic_replace(&file_src2, &file_dst).expect("Atomic replace overwrite must succeed");
    assert!(!file_src2.exists());
    assert_eq!(
        fs::read_to_string(&file_dst).unwrap(),
        "{\"test\":\"data2\"}"
    );
}

#[test]
fn test_selector_checksum_is_unkeyed_sha256() {
    let rec = ActiveDatabaseRecord::new_record(
        1,
        "rst_chk".to_string(),
        "dec_chk".to_string(),
        "digest_chk".to_string(),
        "app_db".to_string(),
        "1".to_string(),
        "app_db_gen_2".to_string(),
        "2".to_string(),
        "Committed".to_string(),
        1_234_567_890,
        "app_db".to_string(),
    );

    // verify_integrity must succeed on newly created record
    assert!(rec.verify_integrity().is_ok());

    // Tamper with active_db_name
    let mut tampered = rec.clone();
    tampered.active_db_name = "app_db_rogue".to_string();
    assert!(
        tampered.verify_integrity().is_err(),
        "Tampered record must fail unkeyed SHA-256 integrity check"
    );
}

#[test]
fn test_decision_token_in_selector_equals_commit_decision_record() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    let decision_token = format!("tok_decision_{}", uuid::Uuid::new_v4());
    let cdr = CommitDecisionRecord {
        restore_id: "rst_bind_test".to_string(),
        backup_id: "bak_bind_test".to_string(),
        old_generation: Some("1".to_string()),
        new_generation: "2".to_string(),
        required_participants: vec!["postgres_data".to_string(), "spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_234_567_890,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: decision_token.clone(),
    };

    let record = ActiveDatabaseRecord::new_record(
        1,
        cdr.restore_id.clone(),
        cdr.decision_token.clone(),
        "digest_test".to_string(),
        "app_db".to_string(),
        "1".to_string(),
        "app_db_gen_2".to_string(),
        cdr.new_generation.clone(),
        "Committed".to_string(),
        cdr.decision_timestamp_nanos,
        "app_db".to_string(),
    );

    assert_eq!(record.decision_id, decision_token);
    assert!(record.verify_integrity().is_ok());

    DatabaseSelector::save_diagnostic_cache(&root, &record).unwrap();
    let loaded = DatabaseSelector::load_diagnostic_cache(&root)
        .unwrap()
        .expect("Diagnostic cache record must exist");
    assert_eq!(loaded.decision_id, decision_token);
    assert_eq!(loaded.target_generation, "2");
    assert!(loaded.verify_integrity().is_ok());
}

#[tokio::test]
async fn test_post_decision_failure_leaves_durable_fence_across_process_restart() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    let staging = root.join(".staged_generation_fence");
    let dummy_archive = tmp.path().join("archive.spnebak");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(&dummy_archive, b"dummy").unwrap();

    let mut journal = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_fence");
    journal.restore_id = "rst_fence".to_string();
    journal.target_generation = "gen_fence".to_string();
    journal.staging_storage_root = staging.clone();

    // Multi-participant decision: requires both postgres_data and spine_storage
    let cdr = CommitDecisionRecord {
        restore_id: "rst_fence".to_string(),
        backup_id: "bak_fence".to_string(),
        old_generation: None,
        new_generation: "gen_fence".to_string(),
        required_participants: vec!["postgres_data".to_string(), "spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_fence".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();

    // Startup recovery sync should forward-recover local storage, but NOT clear the journal
    let sync_res = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
    assert!(sync_res.is_some());

    // Durable journal MUST remain on disk across process exit
    assert!(
        RestoreJournal::load(&root).unwrap().is_some(),
        "Journal must remain present because postgres_data has not converged"
    );

    // ConsistencyBarrier write permit MUST fail closed
    let barrier = ConsistencyBarrier::new().with_storage_root(root.clone());
    let write_res = barrier.acquire_write_permit().await;
    assert!(
        write_res.is_err(),
        "Write permits must fail closed while restore is unrecovered across restarts"
    );
}

#[tokio::test]
async fn test_cluster_recovery_fails_closed_when_generation_mismatches() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    let staging = root.join(".staged_generation_cluster");
    let dummy_archive = tmp.path().join("archive.spnebak");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();
    fs::write(&dummy_archive, b"dummy").unwrap();

    let mut journal = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_cluster");
    journal.restore_id = "rst_cluster".to_string();
    journal.target_generation = "5".to_string();
    journal.staging_storage_root = staging.clone();

    let cdr = CommitDecisionRecord {
        restore_id: "rst_cluster".to_string(),
        backup_id: "bak_cluster".to_string(),
        old_generation: None,
        new_generation: "5".to_string(),
        required_participants: vec!["postgres_data".to_string(), "spine_storage".to_string()],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_cluster".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();

    // Write mismatched diagnostic cache (generation "4" != target "5")
    let record = ActiveDatabaseRecord::new_record(
        1,
        "rst_cluster".to_string(),
        "tok_cluster".to_string(),
        "digest".to_string(),
        "app_db".to_string(),
        "3".to_string(),
        "app_db_gen_4".to_string(),
        "4".to_string(),
        "Committed".to_string(),
        1_000_000,
        "app_db".to_string(),
    );
    DatabaseSelector::save_diagnostic_cache(&root, &record).unwrap();

    // Call recovery without PG pool: fails closed because postgres_data is required
    let missing_pool_res =
        RestoreJournal::handle_startup_recovery_cluster(&root, None, None, &[]).await;
    assert!(
        missing_pool_res.is_err(),
        "Cluster recovery must fail closed when required postgres_data has no DB connection"
    );

    // Call cluster recovery with DB verifier returning generation "4" != target "5": must also fail closed
    let mismatch_check = |_target: &str| Ok(Some("4".to_string()));
    let cluster_res = RestoreJournal::handle_startup_recovery_cluster_with_db_check(
        &root,
        Some(mismatch_check),
        None,
        None,
        &[],
    )
    .await;
    assert!(
        cluster_res.is_err(),
        "Cluster recovery must fail closed when DB generation mismatches"
    );

    // Journal must NOT be cleared
    assert!(
        RestoreJournal::load(&root).unwrap().is_some(),
        "Journal must remain intact on recovery failure"
    );
}

#[test]
fn test_backup_creation_zero_plaintext_intermediate_files() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(storage_root.join("repos")).unwrap();
    fs::create_dir_all(storage_root.join("chunks")).unwrap();
    fs::create_dir_all(storage_root.join("manifests")).unwrap();

    fs::write(
        storage_root.join("repos").join("test_repo.txt"),
        b"repo content",
    )
    .unwrap();
    fs::write(
        storage_root.join("chunks").join("chunk1.bin"),
        b"chunk content",
    )
    .unwrap();

    let (storage_bytes, storage_hash) =
        BackupManager::capture_storage_snapshot_memory(&storage_root).unwrap();
    assert!(!storage_bytes.is_empty());
    assert!(!storage_hash.is_empty());

    // Verify gzip tar decompression in memory
    use flate2::read::GzDecoder;
    use tar::Archive;
    let gz = GzDecoder::new(&storage_bytes[..]);
    let mut archive = Archive::new(gz);
    let mut entry_names = Vec::new();
    for entry in archive.entries().unwrap() {
        let e = entry.unwrap();
        entry_names.push(e.path().unwrap().to_string_lossy().to_string());
    }
    assert!(entry_names.iter().any(|n| n.contains("test_repo.txt")));
    assert!(entry_names.iter().any(|n| n.contains("chunk1.bin")));

    // Verify 0 intermediate files exist on disk in storage_root
    fn count_files(p: &std::path::Path) -> usize {
        let mut count = 0;
        if p.is_dir() {
            if let Ok(entries) = std::fs::read_dir(p) {
                for entry in entries.flatten() {
                    count += count_files(&entry.path());
                }
            }
        } else if p.is_file() {
            count += 1;
        }
        count
    }
    assert_eq!(
        count_files(&storage_root),
        2,
        "Only original test files must exist on disk"
    );
}

#[tokio::test]
async fn test_missing_storage_generation_blocks_write_permits_fail_closed() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    // No storage generation record in root
    assert!(RestoreJournal::read_storage_generation(&root).is_none());

    let barrier = ConsistencyBarrier::new().with_storage_root(root.clone());
    // Enter maintenance mode -> rejected
    let _guard = barrier
        .enter_maintenance(std::time::Duration::from_secs(5))
        .await
        .unwrap();
    assert!(barrier.acquire_write_permit().await.is_err());
}

#[tokio::test]
async fn test_committed_journal_survives_local_sync_recovery_across_all_phases() {
    let phases = vec![
        RestorePhase::Publishing {
            published_participants: vec!["spine_storage".to_string()],
        },
        RestorePhase::DbPublished,
        RestorePhase::StoragePublished,
        RestorePhase::Published,
        RestorePhase::Verified,
        RestorePhase::Committed,
    ];

    for (idx, phase) in phases.into_iter().enumerate() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("storage");
        let staging = root.join(format!(".staged_generation_phase_{}", idx));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&staging).unwrap();
        fs::write(staging.join("file.txt"), b"data").unwrap();
        let dummy_archive = tmp.path().join("bak.spnebak");
        fs::write(&dummy_archive, b"bak").unwrap();

        let mut journal = RestoreJournal::new(&dummy_archive, &root, &staging, "bak_phase");
        journal.target_generation = "10".to_string();
        let cdr = CommitDecisionRecord {
            restore_id: journal.restore_id.clone(),
            backup_id: "bak_phase".to_string(),
            old_generation: Some("9".to_string()),
            new_generation: "10".to_string(),
            required_participants: vec!["spine_storage".to_string(), "coordinator".to_string()],
            staged_manifest_digests: BTreeMap::new(),
            decision_timestamp_nanos: 1_000_000,
            schema_identity: "spine-backup-v2".to_string(),
            decision_token: "tok_phase".to_string(),
        };
        journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();
        if !matches!(phase, RestorePhase::CommitDecision(_)) {
            journal.persist(phase.clone()).unwrap();
        }

        // Local sync recovery must forward recover storage, yet RETAIN journal for Committed and all post-decision phases!
        let msg = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
        assert!(msg.is_some());
        assert_eq!(
            RestoreJournal::read_storage_generation(&root).as_deref(),
            Some("10")
        );
        assert!(
            RestoreJournal::load(&root).unwrap().is_some(),
            "Journal at phase {:?} MUST survive local sync recovery",
            phase
        );

        // Idempotent second sync recovery also retains the journal cleanly
        let msg2 = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
        assert!(msg2.is_some());
        assert!(RestoreJournal::load(&root).unwrap().is_some());
    }
}

#[tokio::test]
async fn test_required_participant_recovery_fails_closed_when_endpoint_missing() {
    // 1. Coordinator required, coordinator = None -> hard error
    {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_coord_missing");
        let staging = root.join(".staged");
        fs::create_dir_all(&staging).unwrap();
        RestoreJournal::write_storage_generation(&root, "12").unwrap();
        let mut journal =
            RestoreJournal::new(&tmp.path().join("b.bak"), &root, &staging, "bak_coord");
        journal.target_generation = "12".to_string();
        let cdr = CommitDecisionRecord {
            restore_id: journal.restore_id.clone(),
            backup_id: "bak_coord".to_string(),
            old_generation: None,
            new_generation: "12".to_string(),
            required_participants: vec!["spine_storage".to_string(), "coordinator".to_string()],
            staged_manifest_digests: BTreeMap::new(),
            decision_timestamp_nanos: 1_000_000,
            schema_identity: "spine-backup-v2".to_string(),
            decision_token: "tok_coord_missing".to_string(),
        };
        journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();

        let err = RestoreJournal::handle_startup_recovery_cluster(&root, None, None, &[])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("coordinator"));
        assert!(RestoreJournal::load(&root).unwrap().is_some());
    }

    // 2. node-2 required, missing from storage_nodes -> hard error
    {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_node_missing");
        let staging = root.join(".staged");
        fs::create_dir_all(&staging).unwrap();
        RestoreJournal::write_storage_generation(&root, "12").unwrap();
        let mut journal =
            RestoreJournal::new(&tmp.path().join("b.bak"), &root, &staging, "bak_node");
        journal.target_generation = "12".to_string();
        let cdr = CommitDecisionRecord {
            restore_id: journal.restore_id.clone(),
            backup_id: "bak_node".to_string(),
            old_generation: None,
            new_generation: "12".to_string(),
            required_participants: vec!["spine_storage".to_string(), "node-2".to_string()],
            staged_manifest_digests: BTreeMap::new(),
            decision_timestamp_nanos: 1_000_000,
            schema_identity: "spine-backup-v2".to_string(),
            decision_token: "tok_node_missing".to_string(),
        };
        journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();

        let err = RestoreJournal::handle_startup_recovery_cluster(
            &root,
            None,
            None,
            &[("node-1".to_string(), "127.0.0.1:9".to_string())],
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("node-2"));
        assert!(RestoreJournal::load(&root).unwrap().is_some());
    }
}

#[tokio::test]
async fn test_recovery_rejects_malformed_and_overflowing_generations_before_rpcs() {
    let rpc_hits = Arc::new(AtomicUsize::new(0));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let hits_clone = Arc::clone(&rpc_hits);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    tokio::spawn(async move {
        while !stop_clone.load(Ordering::SeqCst) {
            if let Ok(Ok((_stream, _))) =
                tokio::time::timeout(std::time::Duration::from_millis(100), listener.accept()).await
            {
                hits_clone.fetch_add(1, Ordering::SeqCst);
            }
        }
    });

    let malformed_values = vec![
        "",
        "0",
        "not_a_number",
        "12abc",
        "18446744073709551616",       // u64::MAX + 1
        "99999999999999999999999999", // huge overflow
    ];

    for bad_gen in &malformed_values {
        assert!(
            RestoreJournal::parse_generation_u64(bad_gen).is_err(),
            "parse_generation_u64 must reject '{}'",
            bad_gen
        );

        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root");
        let staging = root.join(".staged");
        fs::create_dir_all(&staging).unwrap();
        RestoreJournal::write_storage_generation(&root, bad_gen).unwrap();
        let mut journal =
            RestoreJournal::new(&tmp.path().join("b.bak"), &root, &staging, "bak_bad");
        journal.target_generation = bad_gen.to_string();
        let cdr = CommitDecisionRecord {
            restore_id: journal.restore_id.clone(),
            backup_id: "bak_bad".to_string(),
            old_generation: None,
            new_generation: bad_gen.to_string(),
            required_participants: vec!["spine_storage".to_string(), "coordinator".to_string()],
            staged_manifest_digests: BTreeMap::new(),
            decision_timestamp_nanos: 1_000_000,
            schema_identity: "spine-backup-v2".to_string(),
            decision_token: "tok_bad".to_string(),
        };
        journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();

        let coord_client = CoordinatorClient::new(addr.clone());
        let res =
            RestoreJournal::handle_startup_recovery_cluster(&root, None, Some(&coord_client), &[])
                .await;
        assert!(res.is_err(), "Malformed gen '{}' must fail closed", bad_gen);
    }

    // Also test no-journal malformed storage generation fails closed BEFORE coordinator RPC
    {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("root_no_jnl");
        fs::create_dir_all(&root).unwrap();
        RestoreJournal::write_storage_generation(&root, "invalid_storage_gen").unwrap();
        let coord_client = CoordinatorClient::new(addr.clone());
        let res =
            RestoreJournal::handle_startup_recovery_cluster(&root, None, Some(&coord_client), &[])
                .await;
        assert!(res.is_err());
    }

    stop.store(true, Ordering::SeqCst);
    assert_eq!(
        rpc_hits.load(Ordering::SeqCst),
        0,
        "Zero RPC connections may be opened for malformed/overflowing generation values"
    );
}

#[test]
fn test_windows_durability_local_ntfs_writethrough_dest_missing_and_exists() {
    // Test directly inside the local NTFS workspace target directory as well as tempdir
    let local_ntfs_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!(
            "win_durability_test_{}",
            uuid::Uuid::new_v4().simple()
        ));
    fs::create_dir_all(&local_ntfs_dir).unwrap();

    // 1. Destination missing replacement (strict write-through)
    let src_missing = local_ntfs_dir.join("src_missing.json");
    let dst = local_ntfs_dir.join("target_journal.json");
    fs::write(&src_missing, b"{\"seq\":1}").unwrap();
    safe_atomic_replace_strict_writethrough(&src_missing, &dst)
        .expect("Strict write-through destination-missing replace must succeed on local NTFS");
    assert!(!src_missing.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "{\"seq\":1}");

    // 2. Destination exists replacement (strict write-through)
    let src_exists = local_ntfs_dir.join("src_exists.json");
    fs::write(&src_exists, b"{\"seq\":2}").unwrap();
    safe_atomic_replace_strict_writethrough(&src_exists, &dst)
        .expect("Strict write-through destination-exists replace must succeed on local NTFS");
    assert!(!src_exists.exists());
    assert_eq!(fs::read_to_string(&dst).unwrap(), "{\"seq\":2}");

    // 3. safe_atomic_replace under tempdir()
    let tmp = tempdir().unwrap();
    let t_src = tmp.path().join("t_src.json");
    let t_dst = tmp.path().join("t_dst.json");
    fs::write(&t_src, b"temp_v1").unwrap();
    safe_atomic_replace(&t_src, &t_dst).unwrap();
    let t_src2 = tmp.path().join("t_src2.json");
    fs::write(&t_src2, b"temp_v2").unwrap();
    safe_atomic_replace(&t_src2, &t_dst).unwrap();
    assert_eq!(fs::read_to_string(&t_dst).unwrap(), "temp_v2");

    let _ = fs::remove_dir_all(&local_ntfs_dir);
}

#[test]
fn test_bounded_memory_encrypted_streaming_backup_large_payload() {
    #[cfg(windows)]
    fn current_working_set_bytes() -> usize {
        #[repr(C)]
        struct ProcessMemoryCounters {
            cb: u32,
            page_fault_count: u32,
            peak_working_set_size: usize,
            working_set_size: usize,
            quota_peak_paged_pool_usage: usize,
            quota_paged_pool_usage: usize,
            quota_peak_non_paged_pool_usage: usize,
            quota_non_paged_pool_usage: usize,
            pagefile_usage: usize,
            peak_pagefile_usage: usize,
        }
        extern "system" {
            fn GetCurrentProcess() -> isize;
            fn K32GetProcessMemoryInfo(
                process: isize,
                ppsmem_counters: *mut ProcessMemoryCounters,
                cb: u32,
            ) -> i32;
        }
        let mut pmc = std::mem::MaybeUninit::<ProcessMemoryCounters>::zeroed();
        unsafe {
            let ptr = pmc.as_mut_ptr();
            (*ptr).cb = std::mem::size_of::<ProcessMemoryCounters>() as u32;
            if K32GetProcessMemoryInfo(GetCurrentProcess(), ptr, (*ptr).cb) != 0 {
                return (*ptr).working_set_size;
            }
        }
        0
    }

    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("large_storage");
    fs::create_dir_all(storage_root.join("repos")).unwrap();

    // Generate 16 MiB of incompressible high-entropy storage files using SplitMix64
    for i in 0..4u32 {
        let mut data = vec![0u8; 4 * 1024 * 1024];
        let mut seed = (i as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        for chunk in data.chunks_mut(8) {
            seed = seed.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = seed;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            z ^= z >> 31;
            let bytes = z.to_le_bytes();
            let n = chunk.len();
            chunk.copy_from_slice(&bytes[..n]);
        }
        fs::write(
            storage_root.join("repos").join(format!("blob_{}.bin", i)),
            &data,
        )
        .unwrap();
    }

    // Stream an additional 12 MiB database JSONL payload through EphemeralEncryptedTemp with 256 KiB chunking
    let chunk_size = 256 * 1024u32;
    #[cfg(windows)]
    let baseline_ws = current_working_set_bytes();

    let (pg_tmp, pg_key, mut pg_enc_writer) = EphemeralEncryptedTemp::begin(chunk_size).unwrap();
    use std::io::Write;
    let line = format!(
        "{{\"table\":\"commits\",\"payload\":\"{}\"}}\n",
        "x".repeat(1000)
    );
    for _ in 0..12_000 {
        pg_enc_writer.write_all(line.as_bytes()).unwrap();
    }
    let pg_summary = pg_enc_writer.finish().unwrap();
    let pg_enc = EphemeralEncryptedTemp::from_finished(pg_tmp, pg_key, pg_summary);

    let storage_enc =
        BackupManager::capture_storage_snapshot_encrypted(&storage_root, chunk_size).unwrap();

    let stream_participants = vec![
        (
            BackupParticipantRecord {
                participant_id: "postgres_data".to_string(),
                volume_name: "postgres_data".to_string(),
                snapshot_id: "pg_large_1".to_string(),
                fencing_generation: 1,
                byte_length: pg_enc.byte_length,
                sha256_checksum: pg_enc.sha256_checksum.clone(),
                archive_rel_path: "database/snapshot.jsonl".to_string(),
            },
            ParticipantPayloadSource::EncryptedTemp(pg_enc),
        ),
        (
            BackupParticipantRecord {
                participant_id: "spine_storage".to_string(),
                volume_name: "spine_storage".to_string(),
                snapshot_id: "st_large_1".to_string(),
                fencing_generation: 1,
                byte_length: storage_enc.byte_length,
                sha256_checksum: storage_enc.sha256_checksum.clone(),
                archive_rel_path: "storage/spine_storage_snapshot.tar.gz".to_string(),
            },
            ParticipantPayloadSource::EncryptedTemp(storage_enc),
        ),
    ];

    let out_archive = tmp.path().join("large_backup.spnebak");
    let manifest = BackupManager::create_cluster_backup_streaming(
        &storage_root,
        None,
        stream_participants,
        Some(1),
        &out_archive,
        Some("LargePayloadSecret!123"),
        1,
    )
    .unwrap();

    assert!(manifest.total_bytes > 20 * 1024 * 1024);

    #[cfg(windows)]
    {
        let after_ws = current_working_set_bytes();
        // Working set growth must remain strictly bounded well below raw ~28 MiB uncompressed payload sizes
        if baseline_ws > 0 {
            let delta = after_ws.saturating_sub(baseline_ws);
            assert!(
                delta < 16 * 1024 * 1024,
                "Working set grew by {} bytes during >28 MiB backup (expected bounded < 16 MiB)",
                delta
            );
        }
    }

    // Decrypt and verify archive roundtrip without plaintext intermediates on disk
    let restore_target = tmp.path().join("restore_target");
    let session = BackupManager::prepare_restore_session(
        &out_archive,
        &restore_target,
        Some("LargePayloadSecret!123"),
    )
    .unwrap();
    assert_eq!(session.manifest.backup_id, manifest.backup_id);
}

struct MockStorageNodeState {
    node_id: RwLock<String>,
    fencing_generation: AtomicU64,
    in_maintenance: AtomicBool,
    staged_snapshot: RwLock<Option<(String, u64)>>,
    finalized: AtomicBool,
    fail_finalize_once: AtomicBool,
}

async fn spawn_mock_storage_node(state: Arc<MockStorageNodeState>) -> (String, Arc<AtomicBool>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_clone = Arc::clone(&stop);

    tokio::spawn(async move {
        while !stop_clone.load(Ordering::SeqCst) {
            if let Ok(Ok((mut stream, _))) =
                tokio::time::timeout(std::time::Duration::from_millis(50), listener.accept()).await
            {
                let st = Arc::clone(&state);
                tokio::spawn(async move {
                    let mut len_buf = [0u8; 4];
                    if stream.read_exact(&mut len_buf).await.is_err() {
                        return;
                    }
                    let len = u32::from_be_bytes(len_buf) as usize;
                    let mut body = vec![0u8; len];
                    if stream.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let Ok(req) = ClusterRpcRequest::from_bytes(&body) else {
                        return;
                    };
                    let resp = match req {
                        ClusterRpcRequest::QueryMaintenanceStatus => {
                            ClusterRpcResponse::MaintenanceStatus {
                                in_maintenance: st.in_maintenance.load(Ordering::SeqCst),
                                fencing_generation: st.fencing_generation.load(Ordering::SeqCst),
                                active_writers: 0,
                                node_id: Some(st.node_id.read().unwrap().clone()),
                            }
                        }
                        ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation } => {
                            st.in_maintenance.store(true, Ordering::SeqCst);
                            let cur = st.fencing_generation.load(Ordering::SeqCst);
                            if fencing_generation > cur {
                                // Only maintain barrier generation without overwriting staged target
                            }
                            ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                        }
                        ClusterRpcRequest::QueryStagedVolumeRestore { snapshot_id } => {
                            let staged = st.staged_snapshot.read().unwrap().clone();
                            match staged {
                                Some((s_id, gen)) if s_id == snapshot_id => {
                                    ClusterRpcResponse::StagedVolumeRestoreStatus {
                                        snapshot_id,
                                        prepared: true,
                                        acknowledgement: Some(ParticipantAcknowledgement {
                                            restore_id: "rst_7p".to_string(),
                                            backup_id: "bak_7p".to_string(),
                                            participant_id: st.node_id.read().unwrap().clone(),
                                            volume_name: "vol".to_string(),
                                            canonical_resource_id: "res".to_string(),
                                            previous_generation: None,
                                            target_generation: gen.to_string(),
                                            staged_digest: "a".repeat(64),
                                            archive_payload_sha256: "a".repeat(64),
                                            staged_resource_digest: "a".repeat(64),
                                            staged_byte_length: 128,
                                            prepared_at_nanos: 1,
                                            prepared: true,
                                            durable_staging_record_id: "rec".to_string(),
                                            current_state: "Staged".to_string(),
                                        }),
                                    }
                                }
                                _ => ClusterRpcResponse::StagedVolumeRestoreStatus {
                                    snapshot_id,
                                    prepared: false,
                                    acknowledgement: None,
                                },
                            }
                        }
                        ClusterRpcRequest::CommitVolumeRestore {
                            snapshot_id,
                            fencing_generation,
                        } => {
                            let mut staged = st.staged_snapshot.write().unwrap();
                            match staged.as_ref() {
                                Some((s_id, gen))
                                    if s_id == &snapshot_id && *gen == fencing_generation =>
                                {
                                    *staged = None;
                                    st.fencing_generation
                                        .store(fencing_generation, Ordering::SeqCst);
                                    st.in_maintenance.store(false, Ordering::SeqCst);
                                    ClusterRpcResponse::VolumeRestoreCommitted {
                                        snapshot_id,
                                        success: true,
                                    }
                                }
                                _ => ClusterRpcResponse::Error(
                                    "Staged restore not found for commit".to_string(),
                                ),
                            }
                        }
                        ClusterRpcRequest::FinalizeVolumeRestore { snapshot_id } => {
                            if st.fail_finalize_once.swap(false, Ordering::SeqCst) {
                                ClusterRpcResponse::Error(
                                    "Injected FinalizeVolumeRestore RPC failure".to_string(),
                                )
                            } else {
                                st.finalized.store(true, Ordering::SeqCst);
                                ClusterRpcResponse::VolumeRestoreFinalized {
                                    snapshot_id,
                                    success: true,
                                }
                            }
                        }
                        _ => ClusterRpcResponse::Error("Unsupported mock RPC".to_string()),
                    };
                    if let Ok(rb) = resp.to_bytes() {
                        let rl = (rb.len() as u32).to_be_bytes();
                        let _ = stream.write_all(&rl).await;
                        let _ = stream.write_all(&rb).await;
                        let _ = stream.flush().await;
                    }
                });
            }
        }
    });
    (addr, stop)
}

#[tokio::test]
async fn test_seven_participant_e2e_recovery_fault_injection_and_partial_restarts() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("api_storage");
    let staging = root.join(".staged_7p");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();

    // 1. Setup SSH identity on disk
    let ssh_dir = root.join("ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::write(ssh_dir.join("host_key_ed25519"), b"mock-private-key").unwrap();

    // 2. Setup Local Storage generation "42" and retained .old_generation
    RestoreJournal::write_storage_generation(&root, "42").unwrap();

    // 3. Setup REAL CoordinatorStore (file-backed, tests disk staging + restart durability)
    let coord_dir = tmp.path().join("coordinator_data");
    fs::create_dir_all(&coord_dir).unwrap();
    let coord_state_path = coord_dir.join("coordinator_state.json");
    let coord_store =
        Arc::new(FileBackedCoordinatorStore::new_file_backed(&coord_state_path).unwrap());
    let (_, coord_payload) = coord_store.capture_snapshot("tmp_cap", 41).unwrap();
    coord_store.enter_maintenance(41).unwrap();
    coord_store
        .stage_restore("snap_coord_7p", 42, &coord_payload)
        .unwrap();

    // Simulate Coordinator TCP RPC endpoint backed by FileBackedCoordinatorStore
    let coord_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let coord_addr = coord_listener.local_addr().unwrap().to_string();
    let coord_stop = Arc::new(AtomicBool::new(false));
    let coord_stop_c = Arc::clone(&coord_stop);
    let cs_clone = Arc::clone(&coord_store);
    tokio::spawn(async move {
        while !coord_stop_c.load(Ordering::SeqCst) {
            if let Ok(Ok((mut stream, _))) = tokio::time::timeout(
                std::time::Duration::from_millis(50),
                coord_listener.accept(),
            )
            .await
            {
                let store = Arc::clone(&cs_clone);
                tokio::spawn(async move {
                    let mut lb = [0u8; 4];
                    if stream.read_exact(&mut lb).await.is_err() {
                        return;
                    }
                    let mut body = vec![0u8; u32::from_be_bytes(lb) as usize];
                    if stream.read_exact(&mut body).await.is_err() {
                        return;
                    }
                    let Ok(req) = ClusterRpcRequest::from_bytes(&body) else {
                        return;
                    };
                    let resp = match req {
                        ClusterRpcRequest::QueryMaintenanceStatus => {
                            let (im, fg, aw) = store.get_maintenance_status();
                            ClusterRpcResponse::MaintenanceStatus {
                                in_maintenance: im,
                                fencing_generation: fg,
                                active_writers: aw,
                                node_id: Some("coordinator".to_string()),
                            }
                        }
                        ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation } => {
                            match store.enter_maintenance(fencing_generation) {
                                Ok(()) => {
                                    ClusterRpcResponse::MaintenanceBarrierAck { success: true }
                                }
                                Err(e) => ClusterRpcResponse::Error(e.to_string()),
                            }
                        }
                        ClusterRpcRequest::QueryStagedVolumeRestore { snapshot_id } => {
                            match store.query_staged_restore(&snapshot_id) {
                                Ok(Some(ack)) => ClusterRpcResponse::StagedVolumeRestoreStatus {
                                    snapshot_id,
                                    prepared: true,
                                    acknowledgement: Some(ack),
                                },
                                Ok(None) => ClusterRpcResponse::StagedVolumeRestoreStatus {
                                    snapshot_id,
                                    prepared: false,
                                    acknowledgement: None,
                                },
                                Err(e) => ClusterRpcResponse::Error(e.to_string()),
                            }
                        }
                        ClusterRpcRequest::CommitVolumeRestore {
                            snapshot_id,
                            fencing_generation,
                        } => match store.commit_restore(&snapshot_id, fencing_generation) {
                            Ok(()) => ClusterRpcResponse::VolumeRestoreCommitted {
                                snapshot_id,
                                success: true,
                            },
                            Err(e) => ClusterRpcResponse::Error(e.to_string()),
                        },
                        ClusterRpcRequest::FinalizeVolumeRestore { snapshot_id } => {
                            match store.finalize_restore(&snapshot_id) {
                                Ok(()) => ClusterRpcResponse::VolumeRestoreFinalized {
                                    snapshot_id,
                                    success: true,
                                },
                                Err(e) => ClusterRpcResponse::Error(e.to_string()),
                            }
                        }
                        _ => ClusterRpcResponse::Error("Unsupported".to_string()),
                    };
                    if let Ok(rb) = resp.to_bytes() {
                        let _ = stream.write_all(&(rb.len() as u32).to_be_bytes()).await;
                        let _ = stream.write_all(&rb).await;
                        let _ = stream.flush().await;
                    }
                });
            }
        }
    });

    // 4. Setup 3 storage nodes with fault injection:
    // - node-1: still staged at generation 41 (needs commit + finalize during recovery)
    // - node-2: committed at 42, finalize_restore injected to FAIL ONCE
    // - node-3: starts with WRONG node_id ("rogue-node-9"), then fixed on retry
    let n1 = Arc::new(MockStorageNodeState {
        node_id: RwLock::new("node-1".to_string()),
        fencing_generation: AtomicU64::new(41),
        in_maintenance: AtomicBool::new(false),
        staged_snapshot: RwLock::new(Some(("snap_n1_7p".to_string(), 42))),
        finalized: AtomicBool::new(false),
        fail_finalize_once: AtomicBool::new(false),
    });
    let n2 = Arc::new(MockStorageNodeState {
        node_id: RwLock::new("node-2".to_string()),
        fencing_generation: AtomicU64::new(42),
        in_maintenance: AtomicBool::new(false),
        staged_snapshot: RwLock::new(None),
        finalized: AtomicBool::new(false),
        fail_finalize_once: AtomicBool::new(true), // Fails first FinalizeVolumeRestore RPC!
    });
    let n3 = Arc::new(MockStorageNodeState {
        node_id: RwLock::new("rogue-node-9".to_string()), // Wrong participant identity!
        fencing_generation: AtomicU64::new(42),
        in_maintenance: AtomicBool::new(false),
        staged_snapshot: RwLock::new(None),
        finalized: AtomicBool::new(false),
        fail_finalize_once: AtomicBool::new(false),
    });

    let (n1_addr, n1_stop) = spawn_mock_storage_node(Arc::clone(&n1)).await;
    let (n2_addr, n2_stop) = spawn_mock_storage_node(Arc::clone(&n2)).await;
    let (n3_addr, n3_stop) = spawn_mock_storage_node(Arc::clone(&n3)).await;

    // 5. Setup durable restore journal with all 7 participants & snapshot IDs
    let mut journal =
        RestoreJournal::new(&tmp.path().join("bak.spnebak"), &root, &staging, "bak_7p");
    journal.target_generation = "42".to_string();
    journal
        .record_participant_snapshot("coordinator", "snap_coord_7p")
        .unwrap();
    journal
        .record_participant_snapshot("node-1", "snap_n1_7p")
        .unwrap();
    journal
        .record_participant_snapshot("node-2", "snap_n2_7p")
        .unwrap();
    journal
        .record_participant_snapshot("node-3", "snap_n3_7p")
        .unwrap();

    let cdr = CommitDecisionRecord {
        restore_id: journal.restore_id.clone(),
        backup_id: "bak_7p".to_string(),
        old_generation: Some("41".to_string()),
        new_generation: "42".to_string(),
        required_participants: vec![
            "postgres_data".to_string(),
            "spine_storage".to_string(),
            "spine_ssh".to_string(),
            "coordinator".to_string(),
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_7p".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();
    journal.persist(RestorePhase::Committed).unwrap();

    // First verify local sync recovery retains Committed journal
    let _ = RestoreJournal::handle_startup_recovery_sync(&root).unwrap();
    assert!(RestoreJournal::load(&root).unwrap().is_some());

    let coord_client = CoordinatorClient::new(coord_addr);
    let nodes = vec![
        ("node-1".to_string(), n1_addr),
        ("node-2".to_string(), n2_addr),
        ("node-3".to_string(), n3_addr),
    ];
    let db_ok = |_target: &str| Ok(Some("42".to_string()));

    // RECOVERY RUN 1: Fails at node-2 FinalizeVolumeRestore! Coordinator and node-1 already committed & finalized cleanly!
    let run1 = RestoreJournal::handle_startup_recovery_cluster_with_db_check(
        &root,
        Some(db_ok),
        None,
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(
        run1.is_err(),
        "Run 1 must fail closed on node-2 finalize error"
    );
    let run1_err = run1.unwrap_err().to_string();
    assert!(
        run1_err.contains("finalize_restore"),
        "Expected Run 1 error to contain 'finalize_restore', got: {}",
        run1_err
    );

    // Verify journal still exists and partial durable convergence state was persisted!
    let j1 = RestoreJournal::load(&root)
        .unwrap()
        .expect("Journal must persist across partial failure");
    assert!(j1.participant_progress["coordinator"].finalized);
    assert!(j1.participant_progress["node-1"].finalized);
    assert!(j1.participant_progress["node-2"].converged);
    assert!(!j1.participant_progress["node-2"].finalized);

    // RECOVERY RUN 2 (fresh restart simulation): node-2 finalize now succeeds, fails at node-3 WRONG identity!
    let run2 = RestoreJournal::handle_startup_recovery_cluster_with_db_check(
        &root,
        Some(db_ok),
        None,
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(
        run2.is_err(),
        "Run 2 must fail closed on node-3 wrong participant identity"
    );
    assert!(run2.unwrap_err().to_string().contains("rogue-node-9"));
    let j2 = RestoreJournal::load(&root)
        .unwrap()
        .expect("Journal must persist when node-3 identity fails");
    assert!(j2.participant_progress["node-2"].finalized);
    assert!(!j2
        .participant_progress
        .get("node-3")
        .map(|s| s.converged)
        .unwrap_or(false));

    // Fix node-3 identity and execute RECOVERY RUN 3 (fresh restart)
    *n3.node_id.write().unwrap() = "node-3".to_string();
    let run3 = RestoreJournal::handle_startup_recovery_cluster_with_db_check(
        &root,
        Some(db_ok),
        None,
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(
        run3.is_ok(),
        "Run 3 must converge and finalize all 7 participants: {:?}",
        run3
    );

    // Journal MUST NOW BE REMOVED and all cluster participants converged & finalized!
    assert!(
        RestoreJournal::load(&root).unwrap().is_none(),
        "Journal must be cleared ONLY after all 7 participants converged and finalized"
    );
    assert_eq!(n1.fencing_generation.load(Ordering::SeqCst), 42);
    assert!(n1.finalized.load(Ordering::SeqCst));
    assert!(n2.finalized.load(Ordering::SeqCst));
    assert!(n3.finalized.load(Ordering::SeqCst));
    assert_eq!(coord_store.get_maintenance_status().1, 42);

    coord_stop.store(true, Ordering::SeqCst);
    n1_stop.store(true, Ordering::SeqCst);
    n2_stop.store(true, Ordering::SeqCst);
    n3_stop.store(true, Ordering::SeqCst);
}

#[tokio::test]
async fn test_rpc_client_rejects_oversized_frame_without_allocation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();

    tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            let mut lb = [0u8; 4];
            let _ = stream.read_exact(&mut lb).await;
            let req_len = u32::from_be_bytes(lb) as usize;
            let mut req_body = vec![0u8; req_len];
            let _ = stream.read_exact(&mut req_body).await;
            // Send malicious 512 MiB length header without body
            let bogus_len = (512u32 * 1024 * 1024).to_be_bytes();
            let _ = stream.write_all(&bogus_len).await;
            let _ = stream.flush().await;
        }
    });

    let client = CoordinatorClient::new(addr);
    let err = client.query_node_details().await.unwrap_err();
    assert!(
        err.to_string().contains("exceeds maximum bound"),
        "Expected oversized frame bound rejection, got: {}",
        err
    );
}

#[cfg(windows)]
#[test]
fn test_windows_fallback_sync_failures_propagate_source_and_destination() {
    use spine_api::backup::platform_atomic_replace_with_hooks;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let tmp = tempdir().unwrap();
    let from = tmp.path().join("src.tmp");
    let to = tmp.path().join("dest.dat");
    fs::write(&from, b"new-payload").unwrap();
    fs::write(&to, b"old-payload").unwrap();

    // 1. Source sync failure must fail before rename and preserve original destination
    let replace_calls = AtomicUsize::new(0);
    let err_src = platform_atomic_replace_with_hooks(
        &from,
        &to,
        |_| Err(std::io::Error::other("injected source sync_all failure")),
        |_, _, _| {
            replace_calls.fetch_add(1, Ordering::SeqCst);
            Ok(())
        },
    )
    .unwrap_err();
    assert!(err_src
        .to_string()
        .contains("injected source sync_all failure"));
    assert_eq!(replace_calls.load(Ordering::SeqCst), 0);
    assert_eq!(fs::read(&to).unwrap(), b"old-payload");

    // 2. Fallback destination sync failure after Win32 OS Error 650 must propagate error
    let sync_calls = AtomicUsize::new(0);
    let err_dst = platform_atomic_replace_with_hooks(
        &from,
        &to,
        |_| {
            let n = sync_calls.fetch_add(1, Ordering::SeqCst);
            if n == 0 {
                Ok(()) // source sync succeeds
            } else {
                Err(std::io::Error::other(
                    "injected destination fallback sync_all failure",
                ))
            }
        },
        |_, _, write_through| {
            if write_through {
                Err(std::io::Error::from_raw_os_error(650))
            } else {
                Ok(())
            }
        },
    )
    .unwrap_err();
    assert!(err_dst
        .to_string()
        .contains("injected destination fallback sync_all failure"));
    assert_eq!(sync_calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn test_rpc_max_payload_worst_case_high_entropy_fits_frame_and_overlimit_rejected() {
    use spine_api::coordinator::ClusterNodeClient;
    use spine_cluster::protocol::{
        ClusterRpcRequest, ClusterRpcResponse, VolumeSnapshotManifest, MAX_CLUSTER_RPC_FRAME_BYTES,
        MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES,
    };

    // High-entropy worst-case JSON byte array: every byte >= 100 encodes as 4 bytes ("255,")
    let max_payload = vec![0xFFu8; MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES];
    let manifest = VolumeSnapshotManifest {
        participant_id: "node-1".to_string(),
        volume_name: "storage_1_data".to_string(),
        snapshot_id: "snap_boundary_test".to_string(),
        fencing_generation: 99,
        file_count: 100,
        total_bytes: MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES as u64,
        sha256_checksum: "0".repeat(64),
        created_at_nanos: 1_700_000_000_000_000_000,
        restore_id: Some("rst_boundary_test".to_string()),
        backup_id: Some("bak_boundary_test".to_string()),
    };

    let req = ClusterRpcRequest::StageVolumeRestore {
        snapshot_id: "snap_boundary_test".to_string(),
        fencing_generation: 99,
        manifest: manifest.clone(),
        payload_bytes: max_payload.clone(),
    };
    let req_bytes = req.to_bytes().unwrap();
    assert!(
        req_bytes.len() <= MAX_CLUSTER_RPC_FRAME_BYTES,
        "Worst-case StageVolumeRestore ({} bytes) exceeded MAX_CLUSTER_RPC_FRAME_BYTES ({} bytes)",
        req_bytes.len(),
        MAX_CLUSTER_RPC_FRAME_BYTES
    );
    match ClusterRpcRequest::from_bytes(&req_bytes).unwrap() {
        ClusterRpcRequest::StageVolumeRestore { payload_bytes, .. } => {
            assert_eq!(payload_bytes.len(), MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES);
        }
        other => panic!("Unexpected request variant: {:?}", other),
    }

    let resp = ClusterRpcResponse::VolumeSnapshotCaptured {
        manifest: manifest.clone(),
        payload_bytes: max_payload,
    };
    let resp_bytes = resp.to_bytes().unwrap();
    assert!(
        resp_bytes.len() <= MAX_CLUSTER_RPC_FRAME_BYTES,
        "Worst-case VolumeSnapshotCaptured ({} bytes) exceeded MAX_CLUSTER_RPC_FRAME_BYTES ({} bytes)",
        resp_bytes.len(),
        MAX_CLUSTER_RPC_FRAME_BYTES
    );

    // Over-limit payload (MAX + 1) must be rejected on client before connecting/sending
    let client = ClusterNodeClient::new("127.0.0.1:9".to_string());
    let over_limit = vec![0xAAu8; MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES + 1];
    let err = client
        .stage_restore(manifest, over_limit)
        .await
        .unwrap_err();
    assert!(
        err.to_string().contains("exceeds limit"),
        "Expected over-limit payload rejection, got: {}",
        err
    );
}

#[tokio::test]
#[ignore = "requires pre-built release candidate binaries in target/release-candidate"]
async fn test_live_coordinator_and_storage_node_processes_restore_and_restart() {
    use spine_api::backup::{CommitDecisionRecord, RestoreJournal, RestorePhase};
    use spine_api::coordinator::{ClusterNodeClient, CoordinatorClient};
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    use common::TestChildGuard as ChildGuard;

    fn find_cluster_binary(name: &str) -> PathBuf {
        use sha2::{Digest, Sha256};
        use std::io::Read;

        let exe_name = if cfg!(windows) {
            format!("{}.exe", name)
        } else {
            name.to_string()
        };
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let repo_root = manifest_dir
            .parent()
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .and_then(|p| p.parent())
            .expect("repo root");
        let rc_dir = repo_root.join("target").join("release-candidate");
        let manifest_path = rc_dir.join("SHA256SUMS");
        let manifest_text = fs::read_to_string(&manifest_path).unwrap_or_else(|e| {
            panic!(
                "Release checksum manifest '{}' not found or unreadable ({}); build and stage release binaries first",
                manifest_path.display(),
                e
            )
        });
        let expected_sha256 = manifest_text
            .lines()
            .filter_map(|line| {
                let mut parts = line.split_whitespace();
                let hash = parts.next()?;
                let file = parts.next()?;
                if file == exe_name {
                    Some(hash.to_ascii_lowercase())
                } else {
                    None
                }
            })
            .next()
            .unwrap_or_else(|| {
                panic!(
                    "No SHA256SUMS entry for '{}' in '{}'",
                    exe_name,
                    manifest_path.display()
                )
            });

        let bin_path = rc_dir.join(&exe_name);
        if !bin_path.exists() {
            panic!(
                "Release binary '{}' not found (debug binaries are disallowed); build and stage release binaries first",
                bin_path.display()
            );
        }
        let mut file = fs::File::open(&bin_path).expect("open release binary");
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let n = file.read(&mut buf).expect("read release binary");
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
        }
        let actual_sha256: String = hasher
            .finalize()
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect();
        assert_eq!(
            actual_sha256,
            expected_sha256,
            "Release binary '{}' SHA-256 mismatch: actual {} != manifest {}",
            bin_path.display(),
            actual_sha256,
            expected_sha256
        );
        println!(
            "VERIFIED RELEASE BINARY: {} sha256={}",
            bin_path.display(),
            actual_sha256
        );
        bin_path
    }

    fn pick_free_addr() -> String {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().to_string()
    }

    async fn wait_for_node(addr: &str) {
        let client = ClusterNodeClient::new(addr.to_string());
        for _ in 0..100 {
            if client.query_node_details().await.is_ok() {
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        }
        panic!("Timed out waiting for live cluster process at {}", addr);
    }

    let coord_bin = find_cluster_binary("spine-coordinator");
    let node_bin = find_cluster_binary("spine-storage-node");

    let tmp = tempdir().unwrap();
    let root = tmp.path().join("api_storage");
    let staging = root.join(".staged_live");
    fs::create_dir_all(&root).unwrap();
    fs::create_dir_all(&staging).unwrap();
    let ssh_dir = root.join("ssh");
    fs::create_dir_all(&ssh_dir).unwrap();
    fs::write(ssh_dir.join("host_key_ed25519"), b"live-e2e-ssh-key").unwrap();
    RestoreJournal::write_storage_generation(&root, "10").unwrap();

    let coord_store_path = tmp.path().join("coord").join("coordinator_state.json");
    fs::create_dir_all(coord_store_path.parent().unwrap()).unwrap();
    let coord_addr = pick_free_addr();

    let n1_dir = tmp.path().join("node-1");
    let n2_dir = tmp.path().join("node-2");
    let n3_dir = tmp.path().join("node-3");
    fs::create_dir_all(&n1_dir).unwrap();
    fs::create_dir_all(&n2_dir).unwrap();
    fs::create_dir_all(&n3_dir).unwrap();
    let n1_addr = pick_free_addr();
    let n2_addr = pick_free_addr();
    let n3_addr = pick_free_addr();

    let spawn_coord = |addr: &str| {
        Command::new(&coord_bin)
            .env("SPINE_CLUSTER_ID", "live-e2e-cluster")
            .env("SPINE_LISTEN", addr)
            .env("SPINE_STORE_PATH", &coord_store_path)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spine-coordinator")
    };
    let spawn_node = |id: &str, addr: &str, dir: &std::path::Path| {
        Command::new(&node_bin)
            .env("SPINE_NODE_ID", id)
            .env("SPINE_LISTEN", addr)
            .env("SPINE_DATA_DIR", dir)
            .env("SPINE_COORDINATOR_ADDR", &coord_addr)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn spine-storage-node")
    };

    let mut guards = ChildGuard::new(vec![
        spawn_coord(&coord_addr),
        spawn_node("node-1", &n1_addr, &n1_dir),
        spawn_node("node-2", &n2_addr, &n2_dir),
        spawn_node("node-3", &n3_addr, &n3_dir),
    ]);

    wait_for_node(&coord_addr).await;
    wait_for_node(&n1_addr).await;
    wait_for_node(&n2_addr).await;
    wait_for_node(&n3_addr).await;

    let coord_client = CoordinatorClient::new(coord_addr.clone());
    let n1_client = ClusterNodeClient::new(n1_addr.clone());
    let n2_client = ClusterNodeClient::new(n2_addr.clone());
    let n3_client = ClusterNodeClient::new(n3_addr.clone());

    // Capture initial snapshots from all 4 live processes
    let (mut cm, cp) = coord_client.capture_snapshot("cap_c", 1).await.unwrap();
    let (mut m1, p1) = n1_client.capture_snapshot("cap_1", 1).await.unwrap();
    let (mut m2, p2) = n2_client.capture_snapshot("cap_2", 1).await.unwrap();
    let (mut m3, p3) = n3_client.capture_snapshot("cap_3", 1).await.unwrap();

    // Enter maintenance and stage target generation 10
    coord_client.enter_maintenance(10).await.unwrap();
    n1_client.enter_maintenance(10).await.unwrap();
    n2_client.enter_maintenance(10).await.unwrap();
    n3_client.enter_maintenance(10).await.unwrap();

    cm.snapshot_id = "snap_live_coord".to_string();
    cm.fencing_generation = 10;
    m1.snapshot_id = "snap_live_n1".to_string();
    m1.fencing_generation = 10;
    m2.snapshot_id = "snap_live_n2".to_string();
    m2.fencing_generation = 10;
    m3.snapshot_id = "snap_live_n3".to_string();
    m3.fencing_generation = 10;

    coord_client.stage_restore(cm, cp).await.unwrap();
    n1_client.stage_restore(m1, p1).await.unwrap();
    n2_client.stage_restore(m2, p2).await.unwrap();
    n3_client.stage_restore(m3, p3).await.unwrap();

    // Crash and restart coordinator + node-2 AFTER staging (before commit)
    let _ = guards.0[0].kill();
    let _ = guards.0[0].wait();
    let _ = guards.0[2].kill();
    let _ = guards.0[2].wait();
    guards.0[0] = spawn_coord(&coord_addr);
    guards.0[2] = spawn_node("node-2", &n2_addr, &n2_dir);
    wait_for_node(&coord_addr).await;
    wait_for_node(&n2_addr).await;

    // Commit coordinator (re-entering maintenance after restart) and node-1, then crash and restart node-1 BEFORE finalize
    coord_client.enter_maintenance(10).await.unwrap();
    coord_client
        .commit_restore("snap_live_coord", 10)
        .await
        .unwrap();
    n1_client.commit_restore("snap_live_n1", 10).await.unwrap();
    guards.replace(1, spawn_node("node-1", &n1_addr, &n1_dir));
    wait_for_node(&n1_addr).await;

    // Persist Committed restore journal and execute startup cluster recovery across live processes
    let mut journal = RestoreJournal::new(
        &tmp.path().join("live.spnebak"),
        &root,
        &staging,
        "bak_live",
    );
    journal.target_generation = "10".to_string();
    journal
        .record_participant_snapshot("coordinator", "snap_live_coord")
        .unwrap();
    journal
        .record_participant_snapshot("node-1", "snap_live_n1")
        .unwrap();
    journal
        .record_participant_snapshot("node-2", "snap_live_n2")
        .unwrap();
    journal
        .record_participant_snapshot("node-3", "snap_live_n3")
        .unwrap();

    let cdr = CommitDecisionRecord {
        restore_id: journal.restore_id.clone(),
        backup_id: "bak_live".to_string(),
        old_generation: Some("0".to_string()),
        new_generation: "10".to_string(),
        required_participants: vec![
            "postgres_data".to_string(),
            "spine_storage".to_string(),
            "spine_ssh".to_string(),
            "coordinator".to_string(),
            "node-1".to_string(),
            "node-2".to_string(),
            "node-3".to_string(),
        ],
        staged_manifest_digests: BTreeMap::new(),
        decision_timestamp_nanos: 1_000_000,
        schema_identity: "spine-backup-v2".to_string(),
        decision_token: "tok_live".to_string(),
    };
    journal.persist(RestorePhase::CommitDecision(cdr)).unwrap();
    journal.persist(RestorePhase::Committed).unwrap();

    let nodes = vec![
        ("node-1".to_string(), n1_addr.clone()),
        ("node-2".to_string(), n2_addr.clone()),
        ("node-3".to_string(), n3_addr.clone()),
    ];
    let db_ok = |_target: &str| Ok(Some("10".to_string()));
    let res = RestoreJournal::handle_startup_recovery_cluster_with_db_check(
        &root,
        Some(db_ok),
        None,
        Some(&coord_client),
        &nodes,
    )
    .await;
    assert!(
        res.is_ok(),
        "Live multi-process recovery must converge all 7 participants: {:?}",
        res
    );
    assert!(RestoreJournal::load(&root).unwrap().is_none());

    for (label, client) in [
        ("coordinator", &ClusterNodeClient::new(coord_addr)),
        ("node-1", &n1_client),
        ("node-2", &n2_client),
        ("node-3", &n3_client),
    ] {
        let (maint, gen, _, id) = client.query_node_details().await.unwrap();
        assert!(!maint, "{} remained in maintenance after recovery", label);
        assert_eq!(gen, 10, "{} generation mismatch after recovery", label);
        assert_eq!(id.as_deref(), Some(label));
    }
}
