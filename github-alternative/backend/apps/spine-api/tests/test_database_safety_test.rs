mod common;

use common::{
    ensure_disposable_marker, require_safe_test_db_url, verify_disposable_database,
    DISPOSABLE_MARKER_TABLE,
};
use sqlx::PgPool;

#[tokio::test]
async fn test_disposable_database_safety_contract() {
    let url = require_safe_test_db_url();
    let pool = PgPool::connect(&url).await.expect("connect to db");

    // 1. Test: Without environment opt-in flag, verification MUST fail
    std::env::remove_var("SPINE_ALLOW_DESTRUCTIVE_TESTS");
    let verify_no_env = verify_disposable_database(&pool).await;
    assert!(
        verify_no_env.is_err(),
        "Verification must fail when SPINE_ALLOW_DESTRUCTIVE_TESTS is unset"
    );
    let err_no_env = verify_no_env.unwrap_err();
    assert!(
        err_no_env.contains("SPINE_ALLOW_DESTRUCTIVE_TESTS=1"),
        "Error message must mention missing env flag: {}",
        err_no_env
    );

    // 2. Enable opt-in flag, but drop marker table: verification MUST fail
    std::env::set_var("SPINE_ALLOW_DESTRUCTIVE_TESTS", "1");
    let drop_marker_sql = format!(
        "DROP TABLE IF EXISTS \"{}\" CASCADE",
        DISPOSABLE_MARKER_TABLE
    );
    sqlx::query(&drop_marker_sql)
        .execute(&pool)
        .await
        .expect("drop marker table");

    let verify_no_marker = verify_disposable_database(&pool).await;
    assert!(
        verify_no_marker.is_err(),
        "Verification must fail when marker table is absent, even if database name contains test"
    );
    let err_no_marker = verify_no_marker.unwrap_err();
    assert!(
        err_no_marker.contains("SAFETY ABORT:"),
        "Error must specify SAFETY ABORT: {}",
        err_no_marker
    );
    assert!(
        err_no_marker.contains(DISPOSABLE_MARKER_TABLE),
        "Error must mention marker table name: {}",
        err_no_marker
    );

    // 3. Now provision explicit disposable marker: verification MUST succeed
    ensure_disposable_marker(&pool)
        .await
        .expect("ensure disposable marker");

    let verify_success = verify_disposable_database(&pool).await;
    assert!(
        verify_success.is_ok(),
        "Verification must succeed when marker table and confirmation record are present: {:?}",
        verify_success.err()
    );
}
