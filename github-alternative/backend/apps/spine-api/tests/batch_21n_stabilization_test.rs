use spine_api::backup::consistency::ConsistencyBarrier;
use spine_api::backup::journal::{RestoreJournal, RestorePhase};
use spine_api::backup::participants::{
    compute_directory_staged_resource_digest, LocalStorageParticipantAdapter, SshParticipantAdapter,
};
use spine_api::backup::{safe_atomic_replace, sync_dir};
use spine_api::db::selector::{ActiveDatabaseRecord, DatabaseSelector};
use std::fs;
use std::io::Write;
use tempfile::tempdir;

#[test]
fn test_selector_diagnostic_cache_never_influences_write_authorization() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("storage_root");
    fs::create_dir_all(&root).unwrap();

    // 1. Storage generation is "42"
    RestoreJournal::write_storage_generation(&root, "42").unwrap();

    // 2. Intentionally create a divergent, stale diagnostic cache file pointing to generation "999"
    let stale_record = ActiveDatabaseRecord::new_record(
        999,
        "rst_stale".to_string(),
        "decision_stale".to_string(),
        "a".repeat(64),
        "app_db".to_string(),
        "41".to_string(),
        "app_db_gen_999".to_string(),
        "999".to_string(),
        "Committed".to_string(),
        1_000_000,
        "app_db".to_string(),
    );
    DatabaseSelector::save_diagnostic_cache(&root, &stale_record).unwrap();

    // 3. Consistency barrier write permit acquisition must ignore the diagnostic cache
    let barrier = ConsistencyBarrier::new().with_storage_root(root.clone());
    let permit = barrier.acquire_write_permit_sync();
    assert!(
        permit.is_ok(),
        "Write authorization must NEVER be influenced by the non-authoritative filesystem diagnostic cache"
    );

    // 4. Now create an active restore journal in Pre-Decision phase
    let dummy_archive = tmp.path().join("dummy.spnebak");
    std::fs::write(&dummy_archive, b"dummy").unwrap();
    let staging_root = tmp.path().join("staging");
    let mut journal = RestoreJournal::new(&dummy_archive, &root, &staging_root, "bak_42");
    journal.persist(RestorePhase::Prepared).unwrap();

    // 5. Active restore journal MUST cause write authorization to fail closed
    let blocked_permit = barrier.acquire_write_permit_sync();
    assert!(
        blocked_permit.is_err(),
        "Write authorization must fail closed during active restore sessions"
    );
}

#[test]
fn test_participant_actual_content_revalidation_detects_post_ack_tampering() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("vcs_root");
    fs::create_dir_all(&root).unwrap();

    RestoreJournal::write_storage_generation(&root, "1").unwrap();

    // Create a mock tar.gz payload containing repo objects
    let tar_gz_bytes = {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let enc = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(enc);

        let data = b"blob 12\0Hello World!";
        let mut header = tar::Header::new_gnu();
        header.set_path("objects/42/test_object").unwrap();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append(&header, &data[..]).unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    };

    let adapter = LocalStorageParticipantAdapter::prepare(
        &root,
        "rst_tamper_test",
        "bak_init",
        "2",
        &tar_gz_bytes,
    )
    .expect("prepare must succeed");

    // Revalidation immediately after preparation must succeed
    assert!(
        adapter.revalidate_staged_contents().is_ok(),
        "Revalidation of pristine staged files must succeed"
    );

    // Tamper with a staged file on disk (post-preparation tampering attack)
    let staged_file = adapter.staged_dir.join("objects/42/test_object");
    assert!(staged_file.exists());
    {
        let mut f = fs::OpenOptions::new()
            .append(true)
            .open(&staged_file)
            .unwrap();
        f.write_all(b"ROGUE_INJECTED_BYTES").unwrap();
    }

    // Revalidation MUST FAIL closed because actual file digest on disk no longer matches acknowledgement!
    let err = adapter.revalidate_staged_contents().unwrap_err();
    assert!(
        err.to_string().contains("digest mismatch"),
        "Post-preparation file tampering must be detected and rejected by revalidate_staged_contents"
    );

    // Remove the file completely -> revalidation MUST FAIL closed
    fs::remove_file(&staged_file).unwrap();
    let missing_err = adapter.revalidate_staged_contents().unwrap_err();
    println!("DEBUG: missing_err={:?}", missing_err.to_string());
    assert!(
        missing_err.to_string().contains("digest mismatch")
            || missing_err.to_string().contains("byte length mismatch")
            || missing_err.to_string().contains("0 files"),
        "Missing file must cause revalidation failure: {}",
        missing_err
    );
}

#[test]
fn test_ssh_participant_actual_content_revalidation() {
    let tmp = tempdir().unwrap();
    let root = tmp.path().join("ssh_root");
    fs::create_dir_all(&root).unwrap();

    // Create a mock tar.gz containing SSH host keys
    let tar_gz_bytes = {
        use flate2::write::GzEncoder;
        use flate2::Compression;
        let enc = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(enc);

        let priv_key =
            b"-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----";
        let mut header = tar::Header::new_gnu();
        header.set_path("ssh_host_ed25519_key").unwrap();
        header.set_size(priv_key.len() as u64);
        header.set_mode(0o600);
        header.set_cksum();
        tar.append(&header, &priv_key[..]).unwrap();

        let pub_key = b"ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAA mock_key";
        let mut pub_header = tar::Header::new_gnu();
        pub_header.set_path("ssh_host_ed25519_key.pub").unwrap();
        pub_header.set_size(pub_key.len() as u64);
        pub_header.set_mode(0o644);
        pub_header.set_cksum();
        tar.append(&pub_header, &pub_key[..]).unwrap();

        tar.into_inner().unwrap().finish().unwrap()
    };

    let adapter =
        SshParticipantAdapter::prepare(&root, "rst_ssh_test", "bak_ssh_init", "2", &tar_gz_bytes)
            .expect("SSH adapter prepare must succeed");

    assert!(adapter.revalidate_staged_contents().is_ok());

    // Corrupt one of the key files
    let key_file = adapter.staged_dir.join("ssh_host_ed25519_key");
    fs::write(&key_file, b"").unwrap(); // Empty file

    let err = adapter.revalidate_staged_contents().unwrap_err();
    assert!(err.to_string().contains("empty") || err.to_string().contains("digest mismatch"));
}

#[test]
fn test_directory_staged_resource_digest_determinism() {
    let tmp = tempdir().unwrap();
    let dir_a = tmp.path().join("dir_a");
    let dir_b = tmp.path().join("dir_b");
    fs::create_dir_all(&dir_a).unwrap();
    fs::create_dir_all(&dir_b).unwrap();

    // Create identical files in different order
    fs::write(dir_a.join("z_file.txt"), b"Content Z").unwrap();
    fs::write(dir_a.join("a_file.txt"), b"Content A").unwrap();

    fs::write(dir_b.join("a_file.txt"), b"Content A").unwrap();
    fs::write(dir_b.join("z_file.txt"), b"Content Z").unwrap();

    // Ignored staging metadata files should not affect the digest
    fs::write(
        dir_a.join(".participant_staging_record.json"),
        b"{\"meta\": \"a\"}",
    )
    .unwrap();
    fs::write(
        dir_b.join(".participant_staging_record.json"),
        b"{\"meta\": \"b\"}",
    )
    .unwrap();

    let (digest_a, bytes_a, count_a) = compute_directory_staged_resource_digest(&dir_a).unwrap();
    let (digest_b, bytes_b, count_b) = compute_directory_staged_resource_digest(&dir_b).unwrap();

    assert_eq!(
        digest_a, digest_b,
        "Directory digest must be completely deterministic and order-invariant"
    );
    assert_eq!(bytes_a, bytes_b);
    assert_eq!(count_a, 2);
    assert_eq!(count_b, 2);
}

#[test]
fn test_platform_atomic_replace_and_readback_persistence() {
    let tmp = tempdir().unwrap();
    let dir = tmp.path().join("atomic_dir");
    fs::create_dir_all(&dir).unwrap();

    let target = dir.join("important_config.json");
    let initial_data = b"{\"version\": 1, \"authoritative\": true}";
    fs::write(&target, initial_data).unwrap();

    let replacement_tmp = dir.join(".tmp_replace.json");
    let updated_data = b"{\"version\": 2, \"authoritative\": true, \"updated\": 42}";
    fs::write(&replacement_tmp, updated_data).unwrap();

    // Atomically replace target
    safe_atomic_replace(&replacement_tmp, &target).expect("safe_atomic_replace must succeed");

    assert!(target.exists());
    let readback = fs::read(&target).unwrap();
    assert_eq!(readback, updated_data);
    assert!(!replacement_tmp.exists());

    // Directory sync must verify directory existence and pass
    sync_dir(&dir).expect("sync_dir must succeed on valid directory");
}
