mod common;

use common::{require_safe_test_db_url, HarnessDisposableDb};
use sqlx::PgPool;

#[tokio::test]
async fn test_disposable_database_safety_contract() {
    // 1. Without SPINE_ALLOW_DESTRUCTIVE_TESTS=1, provisioning MUST fail closed
    std::env::remove_var("SPINE_ALLOW_DESTRUCTIVE_TESTS");
    let prov_no_env = HarnessDisposableDb::provision().await;
    assert!(
        prov_no_env.is_err(),
        "Provisioning must fail when SPINE_ALLOW_DESTRUCTIVE_TESTS is unset"
    );
    let err_msg = prov_no_env.err().unwrap();
    assert!(
        err_msg.contains("SPINE_ALLOW_DESTRUCTIVE_TESTS=1"),
        "Error message must specify missing opt-in flag: {}",
        err_msg
    );

    // 2. Enable opt-in flag for testing
    std::env::set_var("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1");

    // 3. Prove: A pre-existing database called `spine_forge_test` is REJECTED
    let preexisting_url = require_safe_test_db_url();
    let preexisting_res = HarnessDisposableDb::verify_preexisting_rejected(&preexisting_url).await;
    assert!(
        preexisting_res.is_err(),
        "Pre-existing database must be rejected: {:?}",
        preexisting_res
    );
    let pre_err = preexisting_res.unwrap_err();
    assert!(
        pre_err.contains("SAFETY ABORT:") && pre_err.contains("Missing harness creation proof"),
        "Error must specify missing proof: {}",
        pre_err
    );

    // 4. Prove: Failed provisioning operation never proceeds to destructive SQL
    std::env::set_var(
        "SPINE_TEST_DATABASE_URL",
        "postgres://invalid_user:invalid_pass@127.0.0.1:1/invalid_db",
    );
    let failed_prov = HarnessDisposableDb::provision().await;
    assert!(failed_prov.is_err(), "Failed provisioning must fail closed");

    // Restore valid URL
    std::env::set_var("SPINE_TEST_DATABASE_URL", &preexisting_url);

    // 5. Prove: Successful unique provisioning by current harness
    let disposable = HarnessDisposableDb::provision()
        .await
        .expect("provision disposable database");
    assert!(disposable.db_name.starts_with("spine_disp_"));

    // 6. Prove: Verification succeeds for this harness-created database
    let verify_self = disposable
        .verify_can_destructively_modify(&disposable.pool)
        .await;
    assert!(
        verify_self.is_ok(),
        "Verification must succeed on its own created database: {:?}",
        verify_self.err()
    );

    // 7. Prove: Existing public tables cannot be dropped in an unverified database
    let unverified_pool = PgPool::connect(&preexisting_url)
        .await
        .expect("connect preexisting");
    let verify_foreign = disposable
        .verify_can_destructively_modify(&unverified_pool)
        .await;
    assert!(
        verify_foreign.is_err(),
        "Verification MUST fail on an unverified or preexisting database"
    );

    // 8. Prove: Cleanup only drops the uniquely created database, leaving preexisting intact
    let created_db_name = disposable.db_name.clone();
    disposable.cleanup().await.expect("cleanup disposable db");

    // Verify created DB is dropped
    let mut admin_url = url::Url::parse(&preexisting_url).unwrap();
    admin_url.set_path("postgres");
    let admin_pool = PgPool::connect(admin_url.as_str())
        .await
        .expect("connect admin");
    let db_exists: (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&created_db_name)
            .fetch_one(&admin_pool)
            .await
            .expect("check db dropped");
    assert!(
        !db_exists.0,
        "Disposable database must be dropped on cleanup"
    );

    // Verify preexisting DB still exists!
    let preexisting_name = url::Url::parse(&preexisting_url)
        .unwrap()
        .path_segments()
        .unwrap()
        .next()
        .unwrap()
        .to_string();
    let pre_exists: (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM pg_database WHERE datname = $1)")
            .bind(&preexisting_name)
            .fetch_one(&admin_pool)
            .await
            .expect("check preexisting exists");
    assert!(
        pre_exists.0,
        "Pre-existing database must remain completely untouched"
    );
}
