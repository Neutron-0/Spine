mod common;

use common::HarnessDisposableDb;
use spine_api::{
    backup::BackupManager,
    db::{dump_database, is_database_empty, restore_database},
};

#[tokio::test]
async fn test_postgres_dump_and_empty_check() {
    let disp_db = match HarnessDisposableDb::provision_migrated().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test_postgres_dump_and_empty_check: {}", e);
            return;
        }
    };
    let pool = disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

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

    disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_full_archive_backup_restore_roundtrip_with_db_snapshot() {
    let disp_db = match HarnessDisposableDb::provision_migrated().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_full_archive_backup_restore_roundtrip_with_db_snapshot: {}",
                e
            );
            return;
        }
    };
    let pool = disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

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

    disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_postgres_jsonb_numeric_precision_roundtrip() {
    let src_disp_db = match HarnessDisposableDb::provision_migrated().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_postgres_jsonb_numeric_precision_roundtrip: {}",
                e
            );
            return;
        }
    };
    let pool = src_disp_db
        .migrator_pool()
        .await
        .expect("connect source pool");

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

    // Target empty disposable database for restore
    let target_disp_db = HarnessDisposableDb::provision_empty()
        .await
        .expect("provision empty target db");
    let target_pool = target_disp_db
        .migrator_pool()
        .await
        .expect("connect target pool");

    // Verify empty prior to restore
    let empty = is_database_empty(&target_pool).await.expect("check empty");
    assert!(empty, "Database must be empty prior to restore");

    // Restore from streaming reader
    let mut reader = std::io::BufReader::new(std::io::Cursor::new(dump_buf));
    let restore_report = spine_api::restore_database_from_reader(&target_pool, &mut reader)
        .await
        .expect("restore_database_from_reader failed");

    assert!(restore_report.tables_restored > 0);
    assert!(restore_report.rows_restored > 0);

    // Query restored value directly from PostgreSQL
    let row: (String,) =
        sqlx::query_as("SELECT (pinned_repo_ids->>0)::text FROM user_profiles WHERE user_id = $1")
            .bind(&uid)
            .fetch_one(&target_pool)
            .await
            .expect("query restored profile");

    assert_eq!(
        row.0, exact_token,
        "Restored JSONB numeric value MUST match exact token without precision loss"
    );

    src_disp_db.cleanup().await.unwrap();
    target_disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_spine_app_least_privilege_schema_create_rejected() {
    let disp_db = match HarnessDisposableDb::provision_migrated().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_spine_app_least_privilege_schema_create_rejected: {}",
                e
            );
            return;
        }
    };
    let pool = disp_db
        .app_pool()
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

    disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_postgres_lfs_objects_backup_restore_roundtrip() {
    let src_disp_db = match HarnessDisposableDb::provision_migrated().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_postgres_lfs_objects_backup_restore_roundtrip: {}",
                e
            );
            return;
        }
    };
    let pool = src_disp_db
        .migrator_pool()
        .await
        .expect("connect source pool");

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let uid = format!("usr_lfs_{}", uuid::Uuid::new_v4().simple());
    let username = format!("user_{}", &uid[8..16]);
    let email = format!("{}@example.com", &uid[8..16]);

    // Insert user
    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7)",
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

    // Insert repository
    let repo_id = format!("repo_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, 'lfs-test-repo', 'lfs-test-repo', 'test_lfs_repo', true, 'ThreeCopies', $4, $4)",
    )
    .bind(&repo_id)
    .bind(&uid)
    .bind(&username)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert repository");

    // Insert artifact manifest
    let manifest_id = format!("man_{}", uuid::Uuid::new_v4().simple());
    let oid = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 1024, 1024, 1, 'application/octet-stream', '{}', $4)",
    )
    .bind(&manifest_id)
    .bind(oid)
    .bind(oid)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert artifact manifest");

    // Insert lfs_objects mapping
    let lfs_id = format!("lfs_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO lfs_objects (id, repository_id, oid, size_bytes, manifest_id, created_at_nanos) VALUES ($1, $2, $3, 1024, $4, $5)",
    )
    .bind(&lfs_id)
    .bind(&repo_id)
    .bind(oid)
    .bind(&manifest_id)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert lfs_objects");

    // Dump database to streaming writer
    let mut dump_buf = Vec::new();
    spine_api::dump_database_to_writer(&pool, &mut dump_buf)
        .await
        .expect("dump_database_to_writer failed");

    // Restore to target empty DB
    let target_disp_db = HarnessDisposableDb::provision_empty()
        .await
        .expect("provision empty target");
    let target_pool = target_disp_db
        .migrator_pool()
        .await
        .expect("connect target pool");

    let mut reader = std::io::BufReader::new(std::io::Cursor::new(dump_buf));
    let restore_report = spine_api::restore_database_from_reader(&target_pool, &mut reader)
        .await
        .expect("restore_database_from_reader failed");

    assert!(restore_report.tables_restored > 0);

    // Query restored lfs_objects joined with artifact_manifests
    let row: (String, String, String, i64) = sqlx::query_as(
        "SELECT lo.repository_id, lo.oid, m.whole_object_hash, lo.size_bytes FROM lfs_objects lo JOIN artifact_manifests m ON lo.manifest_id = m.id WHERE lo.repository_id = $1 AND lo.oid = $2",
    )
    .bind(&repo_id)
    .bind(oid)
    .fetch_one(&target_pool)
    .await
    .expect("query restored lfs object");

    assert_eq!(row.0, repo_id);
    assert_eq!(row.1, oid);
    assert_eq!(row.2, oid);
    assert_eq!(row.3, 1024);

    src_disp_db.cleanup().await.unwrap();
    target_disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_postgres_multitenant_shared_lfs_backup_restore_roundtrip() {
    let src_disp_db = match HarnessDisposableDb::provision_empty().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_postgres_multitenant_shared_lfs_backup_restore_roundtrip: {}",
                e
            );
            return;
        }
    };
    let pool = src_disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

    spine_api::db::run_migrations(&pool)
        .await
        .expect("run_migrations failed");

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // Insert Alice and Repo A
    let uid_a = format!("usr_a_{}", uuid::Uuid::new_v4().simple());
    let username_a = format!("alice_{}", &uid_a[6..14]);
    let email_a = format!("{}@example.com", username_a);
    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7)",
    )
    .bind(&uid_a)
    .bind(&username_a)
    .bind(username_a.to_lowercase())
    .bind(&email_a)
    .bind(email_a.to_lowercase())
    .bind(&username_a)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let repo_a_id = format!("repo_a_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, 'shared-a', 'shared-a', 'native_a', true, 'ThreeCopies', $4, $4)",
    )
    .bind(&repo_a_id)
    .bind(&uid_a)
    .bind(&username_a)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Insert Bob and Repo B
    let uid_b = format!("usr_b_{}", uuid::Uuid::new_v4().simple());
    let username_b = format!("bob_{}", &uid_b[6..14]);
    let email_b = format!("{}@example.com", username_b);
    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7)",
    )
    .bind(&uid_b)
    .bind(&username_b)
    .bind(username_b.to_lowercase())
    .bind(&email_b)
    .bind(email_b.to_lowercase())
    .bind(&username_b)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let repo_b_id = format!("repo_b_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, 'shared-b', 'shared-b', 'native_b', true, 'ThreeCopies', $4, $4)",
    )
    .bind(&repo_b_id)
    .bind(&uid_b)
    .bind(&username_b)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Insert single shared CAS manifest
    let manifest_id = format!("man_shared_{}", uuid::Uuid::new_v4().simple());
    let oid = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 2048, 1024, 2, 'application/octet-stream', '{}', $4)",
    )
    .bind(&manifest_id)
    .bind(oid)
    .bind(oid)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Both Repo A and Repo B reference the SAME manifest (deduplicated)
    let lfs_a_id = format!("lfs_a_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO lfs_objects (id, repository_id, oid, size_bytes, manifest_id, created_at_nanos) VALUES ($1, $2, $3, 2048, $4, $5)",
    )
    .bind(&lfs_a_id)
    .bind(&repo_a_id)
    .bind(oid)
    .bind(&manifest_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let lfs_b_id = format!("lfs_b_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO lfs_objects (id, repository_id, oid, size_bytes, manifest_id, created_at_nanos) VALUES ($1, $2, $3, 2048, $4, $5)",
    )
    .bind(&lfs_b_id)
    .bind(&repo_b_id)
    .bind(oid)
    .bind(&manifest_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // Dump and restore
    let mut dump_buf = Vec::new();
    spine_api::dump_database_to_writer(&pool, &mut dump_buf)
        .await
        .unwrap();

    let target_disp_db = HarnessDisposableDb::provision_empty().await.unwrap();
    let target_pool = target_disp_db.migrator_pool().await.unwrap();

    let mut reader = std::io::BufReader::new(std::io::Cursor::new(dump_buf));
    spine_api::restore_database_from_reader(&target_pool, &mut reader)
        .await
        .unwrap();

    // Verify both mappings restored and point to same manifest
    let row_a: (String, String, i64, String) = sqlx::query_as(
        "SELECT repository_id, oid, size_bytes, manifest_id FROM lfs_objects WHERE repository_id = $1 AND oid = $2",
    )
    .bind(&repo_a_id)
    .bind(oid)
    .fetch_one(&target_pool)
    .await
    .unwrap();
    assert_eq!(row_a.0, repo_a_id);
    assert_eq!(row_a.1, oid);
    assert_eq!(row_a.2, 2048);
    assert_eq!(row_a.3, manifest_id);

    let row_b: (String, String, i64, String) = sqlx::query_as(
        "SELECT repository_id, oid, size_bytes, manifest_id FROM lfs_objects WHERE repository_id = $1 AND oid = $2",
    )
    .bind(&repo_b_id)
    .bind(oid)
    .fetch_one(&target_pool)
    .await
    .unwrap();
    assert_eq!(row_b.0, repo_b_id);
    assert_eq!(row_b.1, oid);
    assert_eq!(row_b.2, 2048);
    assert_eq!(row_b.3, manifest_id);

    src_disp_db.cleanup().await.unwrap();
    target_disp_db.cleanup().await.unwrap();
}
