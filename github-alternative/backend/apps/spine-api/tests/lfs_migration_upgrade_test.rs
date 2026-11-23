mod common;

use common::HarnessDisposableDb;
use spine_api::db::migrations::{run_migrations, run_migrations_up_to};
use spine_api::db::Database;
use spine_api::db::PgDatabase;
use std::sync::Arc;

#[tokio::test]
async fn test_lfs_migration_fresh_install() {
    let disp_db = match HarnessDisposableDb::provision_empty().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("Skipping test_lfs_migration_fresh_install: {}", e);
            return;
        }
    };
    let pool = disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

    // Fresh install runs all migrations from scratch
    run_migrations(&pool)
        .await
        .expect("all migrations must succeed on clean database");

    let count: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count migrations");
    assert_eq!(count.0, 16, "Fresh install must record all 16 migrations");

    let table_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'lfs_objects')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        table_exists.0,
        "lfs_objects table must exist after fresh install"
    );

    // Verify index exists
    let idx_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE indexname = 'idx_lfs_objects_repo_oid')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        idx_exists.0,
        "idx_lfs_objects_repo_oid index must exist after fresh install"
    );

    // Verify foreign key constraints, table names, indexes, cascade, and restrict contracts
    assert_lfs_objects_schema_and_fk_contracts(&pool).await;

    disp_db.cleanup().await.unwrap();
}

#[tokio::test]
async fn test_lfs_migration_upgrade_safety_and_non_lfs_exclusion() {
    let disp_db = match HarnessDisposableDb::provision_empty().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_lfs_migration_upgrade_safety_and_non_lfs_exclusion: {}",
                e
            );
            return;
        }
    };
    let pool = disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

    // 1. Initialize schema at the prior migration version (0001 through 0015)
    run_migrations_up_to(&pool, "0015_maintenance_lease_owner_token")
        .await
        .expect("run migrations up to 0015");

    let count_before: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count migrations at 0015");
    assert_eq!(
        count_before.0, 15,
        "Baseline schema must have exactly 15 migrations applied"
    );

    // Verify lfs_objects does NOT exist yet at version 15
    let table_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'lfs_objects')",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(
        !table_exists.0,
        "lfs_objects must not exist prior to migration 0016"
    );

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 2. Insert User Alice and Repository A
    let uid_alice = format!("usr_alice_{}", uuid::Uuid::new_v4().simple());
    let username_alice = format!("alice_{}", &uid_alice[10..18]);
    let email_alice = format!("{}@example.com", username_alice);
    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7)",
    )
    .bind(&uid_alice)
    .bind(&username_alice)
    .bind(username_alice.to_lowercase())
    .bind(&email_alice)
    .bind(email_alice.to_lowercase())
    .bind(&username_alice)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert alice");

    let repo_a_id = format!("repo_a_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, 'repo-a', 'repo-a', $4, true, 'ThreeCopies', $5, $5)",
    )
    .bind(&repo_a_id)
    .bind(&uid_alice)
    .bind(&username_alice)
    .bind(format!("native_{}", repo_a_id))
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert repo A");

    // 3. Safety Gate 1: Refusal to infer ownership solely because one repository exists
    // An unmapped manifest with '{}' when only repo A exists MUST FAIL CLOSED!
    let oid_no_meta = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let man_no_meta_id = format!("man_nometa_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 1024, 1024, 1, 'application/octet-stream', '{}', $4)",
    )
    .bind(&man_no_meta_id)
    .bind(oid_no_meta)
    .bind(oid_no_meta)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let mig_res = run_migrations(&pool).await;
    assert!(
        mig_res.is_err(),
        "Migration must fail closed on manifest lacking repository metadata even if only 1 repository exists"
    );
    let err_msg = mig_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("lacks explicit repository_id metadata")
            && err_msg.contains("Refusing to infer repository ownership"),
        "Error message must specify refusal to infer repository ownership: {}",
        err_msg
    );

    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_no_meta_id)
        .execute(&pool)
        .await
        .unwrap();

    // 4. Safety Gate 2: Stale/Deleted repository row
    // Manifest references a repository ID that no longer exists in forge_repositories
    let oid_stale_repo = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    let man_stale_repo_id = format!("man_stale_{}", uuid::Uuid::new_v4().simple());
    let stale_json = serde_json::json!({ "repository_id": "repo_deleted_old" }).to_string();
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 1024, 1024, 1, 'application/octet-stream', $4, $5)",
    )
    .bind(&man_stale_repo_id)
    .bind(oid_stale_repo)
    .bind(oid_stale_repo)
    .bind(&stale_json)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let mig_res = run_migrations(&pool).await;
    assert!(
        mig_res.is_err(),
        "Migration must fail on manifest referencing stale/deleted repository"
    );
    let err_msg = mig_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("does not exist in forge_repositories"),
        "Error message must specify non-existent repository: {}",
        err_msg
    );

    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_stale_repo_id)
        .execute(&pool)
        .await
        .unwrap();

    // 5. Safety Gate 3: Malformed OID (not 64 hex characters)
    let man_invalid_oid_id = format!("man_invoid_{}", uuid::Uuid::new_v4().simple());
    let inv_oid_json = serde_json::json!({ "repository_id": repo_a_id }).to_string();
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, 'short_invalid_oid', 'hash', 1024, 1024, 1, 'application/octet-stream', $2, $3)",
    )
    .bind(&man_invalid_oid_id)
    .bind(&inv_oid_json)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let mig_res = run_migrations(&pool).await;
    assert!(
        mig_res.is_err(),
        "Migration must fail on invalid OID format"
    );
    let err_msg = mig_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("not a valid 64-hex SHA-256 OID"),
        "Error message must specify invalid OID format: {}",
        err_msg
    );

    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_invalid_oid_id)
        .execute(&pool)
        .await
        .unwrap();

    // 6. Non-LFS Artifact Types Exclusion Test:
    // Create non-LFS rows referencing manifests:
    // a) release_assets
    let rel_id = format!("rel_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO releases (id, repo_id, tag_name, target_commit_id, title, author_id, created_at_nanos, updated_at_nanos) VALUES ($1, $2, 'v1.0.0', 'c1', 'Title', $3, $4, $4)",
    )
    .bind(&rel_id)
    .bind(&repo_a_id)
    .bind(&uid_alice)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let man_release_id = format!("man_rel_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, 'app-binary.tar.gz', 'hash', 5000, 1024, 5, 'application/gzip', '{}', $2)",
    )
    .bind(&man_release_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let asset_id = format!("asset_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO release_assets (id, release_id, name, content_type, size_bytes, manifest_id, whole_hash, created_at_nanos) VALUES ($1, $2, 'app-binary.tar.gz', 'application/gzip', 5000, $3, 'hash', $4)",
    )
    .bind(&asset_id)
    .bind(&rel_id)
    .bind(&man_release_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // b) package_versions
    let pkg_id = format!("pkg_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO packages (id, ecosystem, name, owner_type, owner_id, created_at_nanos, updated_at_nanos) VALUES ($1, 'cargo', 'my-crate', 'user', $2, $3, $3)",
    )
    .bind(&pkg_id)
    .bind(&uid_alice)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let man_pkg_id = format!("man_pkg_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, 'my-crate-0.1.0.crate', 'hash', 8000, 1024, 8, 'application/octet-stream', '{}', $2)",
    )
    .bind(&man_pkg_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let ver_id = format!("ver_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO package_versions (id, package_id, version, manifest_id, cksum, size_bytes, metadata_json, created_by_user_id, created_at_nanos) VALUES ($1, $2, '0.1.0', $3, 'cksum', 8000, '{}', $4, $5)",
    )
    .bind(&ver_id)
    .bind(&pkg_id)
    .bind(&man_pkg_id)
    .bind(&uid_alice)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // c) artifact_versions
    let ns_id = format!("ns_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_namespaces (id, name, owner_type, owner_id, created_at_nanos, updated_at_nanos) VALUES ($1, 'my-ns', 'user', $2, $3, $3)",
    )
    .bind(&ns_id)
    .bind(&uid_alice)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let man_art_id = format!("man_art_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, 'blob-v1.bin', 'hash', 3000, 1024, 3, 'application/octet-stream', '{}', $2)",
    )
    .bind(&man_art_id)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let art_ver_id = format!("artver_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_versions (id, namespace_id, artifact_name, version, manifest_id, whole_hash, size_bytes, created_by_user_id, created_at_nanos) VALUES ($1, $2, 'blob', '1.0.0', $3, 'hash', 3000, $4, $5)",
    )
    .bind(&art_ver_id)
    .bind(&ns_id)
    .bind(&man_art_id)
    .bind(&uid_alice)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // 7. Insert VALID legacy LFS manifests with explicit repository_id metadata
    let oid_valid_1 = "1111111111111111111111111111111111111111111111111111111111111111";
    let man_valid_1 = format!("man_v1_{}", uuid::Uuid::new_v4().simple());
    let v1_json = serde_json::json!({ "repository_id": repo_a_id }).to_string();
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 2048, 1024, 2, 'application/octet-stream', $4, $5)",
    )
    .bind(&man_valid_1)
    .bind(oid_valid_1)
    .bind(oid_valid_1)
    .bind(&v1_json)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let oid_valid_2 = "2222222222222222222222222222222222222222222222222222222222222222";
    let man_valid_2 = format!("man_v2_{}", uuid::Uuid::new_v4().simple());
    let v2_json = serde_json::json!({ "repo_id": repo_a_id }).to_string();
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 4096, 1024, 4, 'application/octet-stream', $4, $5)",
    )
    .bind(&man_valid_2)
    .bind(oid_valid_2)
    .bind(oid_valid_2)
    .bind(&v2_json)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // 8. Now apply migration 0016: MUST SUCCEED!
    run_migrations(&pool).await.expect(
        "Migration 0016 must succeed when valid legacy LFS manifests and non-LFS rows coexist",
    );

    // 9. Verify Idempotence: re-running must succeed cleanly
    run_migrations(&pool)
        .await
        .expect("Re-running migrations must be completely idempotent");

    let count_after: (i64,) = sqlx::query_as("SELECT count(*) FROM _schema_migrations")
        .fetch_one(&pool)
        .await
        .expect("count migrations at completion");
    assert_eq!(
        count_after.0, 16,
        "Must have exactly 16 migrations recorded"
    );

    // 10. Verify lfs_objects contains ONLY the 2 valid LFS objects and ZERO non-LFS artifacts!
    let total_lfs_count: (i64,) = sqlx::query_as("SELECT count(*) FROM lfs_objects")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        total_lfs_count.0, 2,
        "lfs_objects must contain exactly the 2 valid LFS manifests and exclude release_assets, package_versions, and artifact_versions"
    );

    // Verify non-LFS manifests are NOT in lfs_objects
    let non_lfs_in_lfs: (i64,) =
        sqlx::query_as("SELECT count(*) FROM lfs_objects WHERE manifest_id IN ($1, $2, $3)")
            .bind(&man_release_id)
            .bind(&man_pkg_id)
            .bind(&man_art_id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        non_lfs_in_lfs.0, 0,
        "Non-LFS manifests must never be mapped into lfs_objects"
    );

    // 11. Verify repository-scoped database lookups using PgDatabase
    let db: Arc<dyn Database> = Arc::new(PgDatabase::new(pool.clone()));

    let found_v1 = db
        .get_lfs_object_by_repo_oid(&repo_a_id, oid_valid_1)
        .await
        .unwrap();
    assert!(found_v1.is_some(), "Valid object 1 must be resolvable");
    assert_eq!(found_v1.unwrap().size_bytes, 2048);

    let found_v2 = db
        .get_lfs_object_by_repo_oid(&repo_a_id, oid_valid_2)
        .await
        .unwrap();
    assert!(found_v2.is_some(), "Valid object 2 must be resolvable");
    assert_eq!(found_v2.unwrap().size_bytes, 4096);

    let manifest_v1 = db
        .get_artifact_manifest_by_repo_oid(&repo_a_id, oid_valid_1)
        .await
        .unwrap();
    assert!(
        manifest_v1.is_some(),
        "Manifest 1 must be resolvable for repo A"
    );
    assert_eq!(manifest_v1.unwrap().total_size, 2048);

    // Verify foreign key constraints, table names, indexes, cascade, and restrict contracts on upgrade
    assert_lfs_objects_schema_and_fk_contracts(&pool).await;

    disp_db.cleanup().await.unwrap();
}

async fn assert_lfs_objects_schema_and_fk_contracts(pool: &sqlx::PgPool) {
    // 1. lfs_objects table exists
    let table_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'lfs_objects')",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(table_exists.0, "lfs_objects table must exist");

    // 2. Indexes exist on lfs_objects
    for idx_name in &[
        "idx_lfs_objects_repo_oid",
        "idx_lfs_objects_manifest",
        "idx_lfs_objects_oid",
    ] {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS (SELECT 1 FROM pg_indexes WHERE tablename = 'lfs_objects' AND indexname = $1)",
        )
        .bind(idx_name)
        .fetch_one(pool)
        .await
        .unwrap();
        assert!(exists.0, "Index {} must exist on lfs_objects", idx_name);
    }

    // 3. Unique constraint uq_repo_lfs_oid exists
    let uq_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (
            SELECT 1 FROM pg_constraint c
            JOIN pg_class t ON c.conrelid = t.oid
            WHERE t.relname = 'lfs_objects' AND c.conname = 'uq_repo_lfs_oid' AND c.contype = 'u'
        )",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert!(
        uq_exists.0,
        "Unique constraint uq_repo_lfs_oid must exist on lfs_objects"
    );

    // 4. FK on repository_id references forge_repositories(id) with CASCADE (confdeltype = 'c')
    let fk_repo: (String, i8) = sqlx::query_as(
        "SELECT f.relname::text, c.confdeltype::\"char\"::int8
         FROM pg_constraint c
         JOIN pg_class t ON c.conrelid = t.oid
         JOIN pg_class f ON c.confrelid = f.oid
         WHERE t.relname = 'lfs_objects' AND c.contype = 'f' AND c.conname LIKE '%repository_id%'",
    )
    .fetch_one(pool)
    .await
    .expect("repository_id foreign key constraint must exist");
    assert_eq!(
        fk_repo.0, "forge_repositories",
        "repository_id FK must reference forge_repositories"
    );
    assert_eq!(
        fk_repo.1, b'c' as i8,
        "repository_id FK must have ON DELETE CASCADE ('c')"
    );

    // 5. FK on manifest_id references artifact_manifests(id) with RESTRICT (confdeltype = 'r')
    let fk_manifest: (String, i8) = sqlx::query_as(
        "SELECT f.relname::text, c.confdeltype::\"char\"::int8
         FROM pg_constraint c
         JOIN pg_class t ON c.conrelid = t.oid
         JOIN pg_class f ON c.confrelid = f.oid
         WHERE t.relname = 'lfs_objects' AND c.contype = 'f' AND c.conname LIKE '%manifest_id%'",
    )
    .fetch_one(pool)
    .await
    .expect("manifest_id foreign key constraint must exist");
    assert_eq!(
        fk_manifest.0, "artifact_manifests",
        "manifest_id FK must reference artifact_manifests"
    );
    assert_eq!(
        fk_manifest.1, b'r' as i8,
        "manifest_id FK must have ON DELETE RESTRICT ('r')"
    );

    // 6. Test live foreign key constraints:
    let uid = format!("u_fk_{}", uuid::Uuid::new_v4().simple());
    let repo_id = format!("r_fk_{}", uuid::Uuid::new_v4().simple());
    let man_id = format!("m_fk_{}", uuid::Uuid::new_v4().simple());
    let lfs_id = format!("l_fk_{}", uuid::Uuid::new_v4().simple());
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, created_at_nanos, updated_at_nanos) VALUES ($1, $1, $1, $1 || '@spine.test', $1 || '@spine.test', $1, 'hash', 'ACTIVE', $2, $2)",
    )
    .bind(&uid)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $2, 'test-fk-repo', 'test-fk-repo', 'native', true, 'ThreeCopies', $3, $3)",
    )
    .bind(&repo_id)
    .bind(&uid)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    let dummy_oid = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, total_size, chunk_count, manifest_json, created_at_nanos) VALUES ($1, $2, 1024, 1, '{}', $3)",
    )
    .bind(&man_id)
    .bind(dummy_oid)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO lfs_objects (id, repository_id, oid, size_bytes, manifest_id, created_at_nanos) VALUES ($1, $2, $3, 1024, $4, $5)",
    )
    .bind(&lfs_id)
    .bind(&repo_id)
    .bind(dummy_oid)
    .bind(&man_id)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    // Assert RESTRICT on manifest: deleting referenced manifest must fail
    let del_man_err = sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_id)
        .execute(pool)
        .await;
    assert!(
        del_man_err.is_err(),
        "Deleting artifact_manifests row referenced by lfs_objects must be rejected by RESTRICT"
    );

    // Assert CASCADE on forge_repositories: deleting repository cascades and removes lfs_objects row!
    sqlx::query("DELETE FROM forge_repositories WHERE id = $1")
        .bind(&repo_id)
        .execute(pool)
        .await
        .unwrap();

    let lfs_rem_count: (i64,) = sqlx::query_as("SELECT count(*) FROM lfs_objects WHERE id = $1")
        .bind(&lfs_id)
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(
        lfs_rem_count.0, 0,
        "Deleting forge_repositories row must CASCADE delete referencing lfs_objects row"
    );

    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_id)
        .execute(pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn test_lfs_migration_preflight_and_operator_remediation() {
    let disp_db = match HarnessDisposableDb::provision_empty().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_lfs_migration_preflight_and_operator_remediation: {}",
                e
            );
            return;
        }
    };
    let pool = disp_db
        .migrator_pool()
        .await
        .expect("connect migrator pool");

    // Initialize schema at 0015
    run_migrations_up_to(&pool, "0015_maintenance_lease_owner_token")
        .await
        .expect("run migrations up to 0015");

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let uid = "u_preflight_test";
    let repo_id = "r_preflight_test";

    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, account_status, created_at_nanos, updated_at_nanos) VALUES ($1, $1, $1, $1 || '@spine.test', $1 || '@spine.test', $1, 'hash', 'ACTIVE', $2, $2)",
    )
    .bind(uid)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $2, 'preflight-repo', 'preflight-repo', 'native', true, 'ThreeCopies', $3, $3)",
    )
    .bind(repo_id)
    .bind(uid)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let oid_ambiguous = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let man_ambiguous = "man_ambiguous_01";
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, total_size, chunk_count, manifest_json, created_at_nanos) VALUES ($1, $2, 1024, 1, '{}', $3)",
    )
    .bind(man_ambiguous)
    .bind(oid_ambiguous)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    // 1. Preflight inventory discovers ambiguous ownership
    let inv1 = spine_api::db::inventory_legacy_lfs_manifests(&pool)
        .await
        .unwrap();
    assert!(!inv1.is_safe_to_migrate);
    assert_eq!(inv1.ambiguous_ownership.len(), 1);
    assert_eq!(inv1.ambiguous_ownership[0].manifest_id, man_ambiguous);

    // 2. Malformed operator input rejected
    let bad_input = vec![spine_api::db::OperatorLfsMapping {
        manifest_id: "nonexistent_manifest".to_string(),
        repository_id: repo_id.to_string(),
    }];
    let err_bad = spine_api::db::remediate_legacy_lfs_manifests(&pool, &bad_input, false).await;
    assert!(err_bad.is_err(), "Non-existent manifest must be rejected");

    let bad_repo = vec![spine_api::db::OperatorLfsMapping {
        manifest_id: man_ambiguous.to_string(),
        repository_id: "nonexistent_repo".to_string(),
    }];
    let err_repo = spine_api::db::remediate_legacy_lfs_manifests(&pool, &bad_repo, false).await;
    assert!(err_repo.is_err(), "Non-existent repo must be rejected");

    // 3. Dry-run remediation succeeds without committing changes
    let valid_mapping = vec![spine_api::db::OperatorLfsMapping {
        manifest_id: man_ambiguous.to_string(),
        repository_id: repo_id.to_string(),
    }];
    let dry_res = spine_api::db::remediate_legacy_lfs_manifests(&pool, &valid_mapping, true)
        .await
        .unwrap();
    assert!(dry_res.is_dry_run);
    assert_eq!(dry_res.manifests_remediated, 1);

    // Verify DB was NOT updated (dry-run rollback)
    let inv2 = spine_api::db::inventory_legacy_lfs_manifests(&pool)
        .await
        .unwrap();
    assert!(!inv2.is_safe_to_migrate);
    assert_eq!(inv2.ambiguous_ownership.len(), 1);

    // 4. Real remediation commits the update
    let real_res = spine_api::db::remediate_legacy_lfs_manifests(&pool, &valid_mapping, false)
        .await
        .unwrap();
    assert!(!real_res.is_dry_run);
    assert_eq!(real_res.manifests_remediated, 1);

    // 5. Rerun is idempotent
    let rerun_res = spine_api::db::remediate_legacy_lfs_manifests(&pool, &valid_mapping, false)
        .await
        .unwrap();
    assert_eq!(rerun_res.already_correct, 1);
    assert_eq!(rerun_res.manifests_remediated, 0);

    // 6. Preflight now reports safe to migrate!
    let inv3 = spine_api::db::inventory_legacy_lfs_manifests(&pool)
        .await
        .unwrap();
    assert!(inv3.is_safe_to_migrate);
    assert_eq!(inv3.ready_to_migrate_count, 1);
    assert_eq!(inv3.ambiguous_ownership.len(), 0);

    // 7. Apply migration 0016
    run_migrations(&pool)
        .await
        .expect("migration 0016 must succeed after preflight remediation");

    // 8. Verify lfs_objects mapping is populated
    let lfs_row: (String, String) =
        sqlx::query_as("SELECT repository_id, oid FROM lfs_objects WHERE manifest_id = $1")
            .bind(man_ambiguous)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(lfs_row.0, repo_id);
    assert_eq!(lfs_row.1, oid_ambiguous);

    disp_db.cleanup().await.unwrap();
}
