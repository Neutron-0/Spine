mod common;

use common::{
    ensure_disposable_marker, require_safe_test_db_url, verify_disposable_database,
    DISPOSABLE_MARKER_TABLE,
};
use spine_api::{
    backup::BackupManager,
    db::{dump_database, is_database_empty, restore_database},
};

#[tokio::test]
async fn test_postgres_dump_and_empty_check() {
    let url = require_safe_test_db_url();
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test database");

    // Ensure baseline schema is present in test database
    spine_api::db::migrations::run_migrations(&pool)
        .await
        .expect("run_migrations on test database failed");

    // 1. Dump database snapshot
    let snapshot_json = dump_database(&pool).await.expect("dump_database failed");
    assert!(
        snapshot_json.contains("\"version\":\"3.0\"")
            || snapshot_json.contains("\"version\": \"3.0\"")
            || snapshot_json.contains("\"version\": \"2.0\"")
            || snapshot_json.contains("\"version\":\"2.0\""),
        "Snapshot JSON must contain version 3.0 or 2.0"
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
    let url = require_safe_test_db_url();
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test database");

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
    assert!(
        extracted_json.contains("\"version\":\"3.0\"")
            || extracted_json.contains("\"version\": \"3.0\"")
            || extracted_json.contains("\"version\": \"2.0\"")
            || extracted_json.contains("\"version\":\"2.0\""),
        "Extracted snapshot must contain version 3.0 or 2.0"
    );
}

#[tokio::test]
async fn test_postgres_jsonb_numeric_precision_roundtrip() {
    let url = require_safe_test_db_url();
    let pool = sqlx::PgPool::connect(&url)
        .await
        .expect("connect to test database");

    // Run migrations
    spine_api::db::migrations::run_migrations(&pool)
        .await
        .expect("run_migrations on test database failed");

    // Insert user and profile with huge arbitrary numeric precision inside JSONB
    let uid = format!("usr_prec_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let username = format!("user_{}", &uid[9..16]);
    let email = format!("{}@example.com", &uid[9..16]);

    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7) ON CONFLICT (id) DO NOTHING"
    )
    .bind(&uid)
    .bind(&username)
    .bind(username.to_lowercase())
    .bind(&email)
    .bind(email.to_lowercase())
    .bind(&username)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert user");

    // Exact high-precision numeric token: 30 digits of integer part + 9 digits of fractional part
    let exact_token = "123456789012345678901234567890.123456789";
    let jsonb_payload = format!("[{}]", exact_token);

    sqlx::query(
        "INSERT INTO user_profiles (user_id, bio, pinned_repo_ids, updated_at_nanos) VALUES ($1, 'precision test', $2::jsonb, $3) ON CONFLICT (user_id) DO UPDATE SET pinned_repo_ids = $2::jsonb"
    )
    .bind(&uid)
    .bind(&jsonb_payload)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert profile with high precision jsonb");

    // Dump database to streaming writer
    let mut dump_buf = Vec::new();
    spine_api::dump_database_to_writer(&pool, &mut dump_buf)
        .await
        .expect("dump_database_to_writer failed");

    let dump_str = String::from_utf8(dump_buf.clone()).expect("valid utf-8 dump");

    // Verify dump contains the exact token character-for-character
    assert!(
        dump_str.contains(exact_token),
        "Dump output MUST contain exact numeric string token '{}' without floating-point conversion",
        exact_token
    );

    // Now clean the database to simulate empty target, verifying explicit disposable marker first
    ensure_disposable_marker(&pool)
        .await
        .expect("ensure disposable marker");
    verify_disposable_database(&pool)
        .await
        .expect("verify disposable database before destructive table drop");

    let tables: Vec<(String,)> = sqlx::query_as(
        "SELECT table_name FROM information_schema.tables WHERE table_schema = 'public' AND table_type = 'BASE TABLE' AND table_name != $1"
    )
    .bind(DISPOSABLE_MARKER_TABLE)
    .fetch_all(&pool)
    .await
    .expect("fetch tables");

    for (tbl,) in tables {
        sqlx::query(&format!("DROP TABLE IF EXISTS \"{}\" CASCADE", tbl))
            .execute(&pool)
            .await
            .expect("drop table");
    }

    // Verify empty
    let empty = is_database_empty(&pool).await.expect("check empty");
    assert!(empty, "Database must be empty prior to restore");

    // Restore from streaming reader
    let mut reader = std::io::BufReader::new(std::io::Cursor::new(dump_buf));
    let restore_report = spine_api::restore_database_from_reader(&pool, &mut reader)
        .await
        .expect("restore_database_from_reader failed");

    assert!(restore_report.tables_restored > 0);
    assert!(restore_report.rows_restored > 0);

    // Query restored value directly from PostgreSQL
    let row: (String,) =
        sqlx::query_as("SELECT (pinned_repo_ids->>0)::text FROM user_profiles WHERE user_id = $1")
            .bind(&uid)
            .fetch_one(&pool)
            .await
            .expect("query restored profile");

    assert_eq!(
        row.0, exact_token,
        "Restored JSONB numeric value MUST match exact token without precision loss"
    );
}

#[tokio::test]
async fn test_spine_app_least_privilege_schema_create_rejected() {
    let app_url = common::get_app_role_test_db_url();
    let pool = sqlx::PgPool::connect(&app_url)
        .await
        .expect("connect to test database as spine_app");

    // Attempting to CREATE TABLE as runtime role spine_app MUST fail with permission denied (42501)
    let res = sqlx::query("CREATE TABLE unauthorized_test (id INT)")
        .execute(&pool)
        .await;

    assert!(
        res.is_err(),
        "Runtime application role 'spine_app' MUST NOT have CREATE privilege on schema public"
    );

    let err = res.unwrap_err();
    let err_str = err.to_string();
    assert!(
        err_str.contains("permission denied") || err_str.contains("42501"),
        "PostgreSQL error must be permission denied / 42501: {}",
        err_str
    );
}
