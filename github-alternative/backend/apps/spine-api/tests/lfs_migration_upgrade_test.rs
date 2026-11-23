mod common;

use common::HarnessDisposableDb;
use spine_api::db::migrations::{run_migrations, run_migrations_up_to};
use spine_api::db::Database;
use spine_api::db::PgDatabase;
use std::sync::Arc;

#[tokio::test]
async fn test_lfs_migration_upgrade_path_and_ambiguity_rejection() {
    let disp_db = match HarnessDisposableDb::provision_empty().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!(
                "Skipping test_lfs_migration_upgrade_path_and_ambiguity_rejection: {}",
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

    // 3. Insert representative pre-existing legacy LFS manifests into artifact_manifests:
    // Manifest 1: Explicit repository_id in manifest_json
    let oid1 = "1111111111111111111111111111111111111111111111111111111111111111";
    let man1_id = format!("man_1_{}", uuid::Uuid::new_v4().simple());
    let man1_json = serde_json::json!({
        "repository_id": repo_a_id,
        "note": "legacy explicit repo metadata"
    })
    .to_string();

    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 2048, 1024, 2, 'application/octet-stream', $4, $5)",
    )
    .bind(&man1_id)
    .bind(oid1)
    .bind(oid1)
    .bind(&man1_json)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert legacy manifest 1");

    // Manifest 2: Unambiguous legacy manifest (only repo A exists currently)
    let oid2 = "2222222222222222222222222222222222222222222222222222222222222222";
    let man2_id = format!("man_2_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 4096, 1024, 4, 'application/octet-stream', '{}', $4)",
    )
    .bind(&man2_id)
    .bind(oid2)
    .bind(oid2)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert legacy manifest 2");

    // 4. Test Ambiguity & Invalid Data Handling:
    // Insert Bob and Repository B so multiple repositories exist
    let uid_bob = format!("usr_bob_{}", uuid::Uuid::new_v4().simple());
    let username_bob = format!("bob_{}", &uid_bob[8..16]);
    let email_bob = format!("{}@example.com", username_bob);
    sqlx::query(
        "INSERT INTO users (id, username, normalized_username, email, normalized_email, display_name, password_hash, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, $4, $5, $6, 'hash', $7, $7)",
    )
    .bind(&uid_bob)
    .bind(&username_bob)
    .bind(username_bob.to_lowercase())
    .bind(&email_bob)
    .bind(email_bob.to_lowercase())
    .bind(&username_bob)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert bob");

    let repo_b_id = format!("repo_b_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO forge_repositories (id, owner_user_id, owner_username, name, normalized_name, native_repo_id, is_private, durability_policy, created_at_nanos, updated_at_nanos) VALUES ($1, $2, $3, 'repo-b', 'repo-b', $4, true, 'ThreeCopies', $5, $5)",
    )
    .bind(&repo_b_id)
    .bind(&uid_bob)
    .bind(&username_bob)
    .bind(format!("native_{}", repo_b_id))
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert repo B");

    // Ambiguity Test A: Insert an unmapped LFS manifest without repository metadata when 2 repositories exist
    let oid_ambiguous = "3333333333333333333333333333333333333333333333333333333333333333";
    let man_ambiguous_id = format!("man_amb_{}", uuid::Uuid::new_v4().simple());
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 1024, 1024, 1, 'application/octet-stream', '{}', $4)",
    )
    .bind(&man_ambiguous_id)
    .bind(oid_ambiguous)
    .bind(oid_ambiguous)
    .bind(now)
    .execute(&pool)
    .await
    .expect("insert ambiguous manifest");

    // Migration MUST fail with clear error rather than assigning to the wrong repository!
    let mig_res = run_migrations(&pool).await;
    assert!(
        mig_res.is_err(),
        "Migration must fail on ambiguous legacy row when multiple repositories exist without explicit repository_id metadata"
    );
    let err_msg = mig_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("Ambiguous legacy LFS manifest")
            && err_msg.contains("multiple repositories exist"),
        "Error message must clearly report ambiguous legacy LFS manifest: {}",
        err_msg
    );

    // Clean up ambiguous manifest to test next safety gate
    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_ambiguous_id)
        .execute(&pool)
        .await
        .unwrap();

    // Ambiguity Test B: Manifest referencing a non-existent repository ID
    let oid_invalid_repo = "4444444444444444444444444444444444444444444444444444444444444444";
    let man_invalid_repo_id = format!("man_inv_{}", uuid::Uuid::new_v4().simple());
    let inv_json = serde_json::json!({ "repository_id": "repo_nonexistent_ghost" }).to_string();
    sqlx::query(
        "INSERT INTO artifact_manifests (id, artifact_id, whole_object_hash, total_size, chunk_size, chunk_count, content_type, manifest_json, created_at_nanos) VALUES ($1, $2, $3, 1024, 1024, 1, 'application/octet-stream', $4, $5)",
    )
    .bind(&man_invalid_repo_id)
    .bind(oid_invalid_repo)
    .bind(oid_invalid_repo)
    .bind(&inv_json)
    .bind(now)
    .execute(&pool)
    .await
    .unwrap();

    let mig_res = run_migrations(&pool).await;
    assert!(
        mig_res.is_err(),
        "Migration must fail when referenced repository does not exist"
    );
    let err_msg = mig_res.unwrap_err().to_string();
    assert!(
        err_msg.contains("does not exist in forge_repositories"),
        "Error message must specify non-existent repository: {}",
        err_msg
    );

    sqlx::query("DELETE FROM artifact_manifests WHERE id = $1")
        .bind(&man_invalid_repo_id)
        .execute(&pool)
        .await
        .unwrap();

    // Ambiguity Test C: Invalid OID format
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

    // 5. Now execute migration 0016 on valid representative legacy data -> MUST SUCCEED!
    run_migrations(&pool)
        .await
        .expect("Migration 0016 must succeed on valid legacy manifests");

    // 6. Test Idempotence: re-running migrations must succeed without error
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

    // 7. Verify lfs_objects was populated correctly for repo A and NOT repo B
    let lfs1: (String, String, i64, String) = sqlx::query_as(
        "SELECT repository_id, oid, size_bytes, manifest_id FROM lfs_objects WHERE repository_id = $1 AND oid = $2",
    )
    .bind(&repo_a_id)
    .bind(oid1)
    .fetch_one(&pool)
    .await
    .expect("lfs object 1 must exist for repo A");
    assert_eq!(lfs1.0, repo_a_id);
    assert_eq!(lfs1.1, oid1);
    assert_eq!(lfs1.2, 2048);
    assert_eq!(lfs1.3, man1_id);

    let lfs2: (String, String, i64, String) = sqlx::query_as(
        "SELECT repository_id, oid, size_bytes, manifest_id FROM lfs_objects WHERE repository_id = $1 AND oid = $2",
    )
    .bind(&repo_a_id)
    .bind(oid2)
    .fetch_one(&pool)
    .await
    .expect("lfs object 2 must exist for repo A");
    assert_eq!(lfs2.0, repo_a_id);
    assert_eq!(lfs2.1, oid2);
    assert_eq!(lfs2.2, 4096);
    assert_eq!(lfs2.3, man2_id);

    // Verify neither object is mapped to Repo B (prevent cross-tenant leak)
    let repo_b_count: (i64,) =
        sqlx::query_as("SELECT count(*) FROM lfs_objects WHERE repository_id = $1")
            .bind(&repo_b_id)
            .fetch_one(&pool)
            .await
            .expect("count repo B objects");
    assert_eq!(
        repo_b_count.0, 0,
        "No objects should be mapped to repository B"
    );

    // 8. Verify repository-scoped database lookups using PgDatabase
    let db: Arc<dyn Database> = Arc::new(PgDatabase::new(pool.clone()));

    let found_a1 = db
        .get_lfs_object_by_repo_oid(&repo_a_id, oid1)
        .await
        .unwrap();
    assert!(found_a1.is_some(), "Object 1 must be resolvable for repo A");
    let found_b1 = db
        .get_lfs_object_by_repo_oid(&repo_b_id, oid1)
        .await
        .unwrap();
    assert!(
        found_b1.is_none(),
        "Object 1 must NOT be resolvable for repo B"
    );

    let manifest_a1 = db
        .get_artifact_manifest_by_repo_oid(&repo_a_id, oid1)
        .await
        .unwrap();
    assert!(
        manifest_a1.is_some(),
        "Manifest 1 must be resolvable for repo A"
    );
    assert_eq!(manifest_a1.unwrap().total_size, 2048);

    let manifest_b1 = db
        .get_artifact_manifest_by_repo_oid(&repo_b_id, oid1)
        .await
        .unwrap();
    assert!(
        manifest_b1.is_none(),
        "Manifest 1 must NOT be resolvable for repo B"
    );

    disp_db.cleanup().await.unwrap();
}
