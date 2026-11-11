use spine_api::backup::journal::{CommitDecisionRecord, RestoreJournal, RestorePhase};
use spine_api::backup::{safe_atomic_replace, sync_dir};
use spine_api::db::selector::ActiveDatabaseRecord;
use std::collections::BTreeMap;
use std::fs;
use tempfile::tempdir;

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
