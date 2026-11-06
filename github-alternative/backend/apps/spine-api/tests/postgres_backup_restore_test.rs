mod common;

use spine_api::{
    backup::BackupManager,
    db::{dump_database, is_database_empty, restore_database},
};

#[tokio::test]
async fn test_postgres_dump_and_empty_check() {
    let Some(url) = common::get_safe_test_db_url() else {
        eprintln!("Skipping postgres_dump_and_empty_check: SPINE_TEST_DATABASE_URL not set");
        return;
    };
    let pool = match sqlx::PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Skipping postgres_dump_and_empty_check: PostgreSQL unreachable ({})",
                e
            );
            return;
        }
    };

    // Ensure baseline schema is present in test database
    spine_api::db::migrations::run_migrations(&pool)
        .await
        .expect("run_migrations on test database failed");

    // 1. Dump database snapshot
    let snapshot_json = dump_database(&pool).await.expect("dump_database failed");
    assert!(
        snapshot_json.contains("\"version\": \"2.0\""),
        "Snapshot JSON must contain version 2.0"
    );
    assert!(
        snapshot_json.contains("\"tables\""),
        "Snapshot JSON must contain tables"
    );

    // 2. Check is_database_empty on populated database -> must be false
    let is_empty = is_database_empty(&pool)
        .await
        .expect("is_database_empty failed");
    assert!(
        !is_empty,
        "Test database has migrated tables/rows, so is_database_empty must be false"
    );

    // 3. Attempting restore on non-empty database must fail with Conflict (Audit Finding F-01 requirement)
    let restore_res = restore_database(&pool, &snapshot_json).await;
    assert!(
        restore_res.is_err(),
        "Restoring onto a non-empty database must fail with an error"
    );
    let err_str = restore_res.unwrap_err().to_string();
    assert!(
        err_str.contains("Target database is not empty"),
        "Error message must specify non-empty database rejection: {}",
        err_str
    );
}

#[tokio::test]
async fn test_full_archive_backup_restore_roundtrip_with_db_snapshot() {
    let Some(url) = common::get_safe_test_db_url() else {
        eprintln!(
            "Skipping full_archive_backup_restore_roundtrip: SPINE_TEST_DATABASE_URL not set"
        );
        return;
    };
    let pool = match sqlx::PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Skipping full_archive_backup_restore_roundtrip: PostgreSQL unreachable ({})",
                e
            );
            return;
        }
    };

    let temp_dir = tempfile::tempdir().unwrap();
    let storage_root = temp_dir.path().join("storage_node");
    std::fs::create_dir_all(&storage_root).unwrap();

    // Create synthetic files in storage root
    std::fs::write(
        storage_root.join("config.toml"),
        "cluster_name = 'spine_prod'\n",
    )
    .unwrap();
    let repo_dir = storage_root.join("repos").join("test_repo");
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join("spine.db"), b"mock spine repository data").unwrap();

    // 1. Capture DB dump
    let db_dump = dump_database(&pool).await.expect("dump_database failed");

    // 2. Create encrypted backup archive with db snapshot
    let archive_path = temp_dir.path().join("cluster_backup.spinebak");
    let passphrase = "integration_test_secret_passphrase_!@#";

    let manifest = BackupManager::create_encrypted_backup(
        &storage_root,
        Some(&db_dump),
        &archive_path,
        passphrase,
        2,
    )
    .expect("create_encrypted_backup failed");

    assert!(manifest.total_files >= 2);
    assert_eq!(manifest.integrity_status, "VERIFIED");

    // 3. Restore archive into a clean destination directory
    let restore_target = temp_dir.path().join("restored_storage");
    let (restored_manifest, report, extracted_db) =
        BackupManager::restore_encrypted_backup(&archive_path, &restore_target, passphrase)
            .expect("restore_encrypted_backup failed");

    assert_eq!(manifest.backup_id, restored_manifest.backup_id);
    assert!(report.integrity_verified);
    assert_eq!(report.files_restored, manifest.total_files);

    // Verify file contents restored correctly
    assert_eq!(
        std::fs::read_to_string(restore_target.join("config.toml")).unwrap(),
        "cluster_name = 'spine_prod'\n"
    );
    assert_eq!(
        std::fs::read(restore_target.join("repos/test_repo/spine.db")).unwrap(),
        b"mock spine repository data"
    );

    // Verify DB snapshot was extracted from the archive
    assert!(
        extracted_db.is_some(),
        "Database snapshot JSON must be present in restore result"
    );
    let extracted_json = extracted_db.unwrap();
    assert!(extracted_json.contains("\"version\": \"2.0\""));
}
