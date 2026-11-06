mod common;

use common::get_safe_test_db_url;
use sqlx::Row;

#[tokio::test]
async fn test_spine_app_role_has_no_superuser_privileges() {
    let url = match get_safe_test_db_url() {
        Some(u) => u,
        None => {
            eprintln!("SKIPPED: test requires SPINE_TEST_DATABASE_URL to be set.");
            return;
        }
    };
    let pool = match sqlx::PgPool::connect(&url).await {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "Skipping least privilege test: PostgreSQL unreachable ({})",
                e
            );
            return;
        }
    };

    // 1. Verify rolsuper, rolcreaterole, rolcreatedb are FALSE for spine_app in pg_roles
    let role_row = sqlx::query(
        "SELECT rolname, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls FROM pg_roles WHERE rolname = 'spine_app'",
    )
    .fetch_one(&pool)
    .await
    .expect("query pg_roles for spine_app");

    let rolsuper: bool = role_row.get("rolsuper");
    let rolcreaterole: bool = role_row.get("rolcreaterole");
    let rolcreatedb: bool = role_row.get("rolcreatedb");
    let rolbypassrls: bool = role_row.get("rolbypassrls");

    assert!(
        !rolsuper,
        "Finding F-04: spine_app MUST NOT possess SUPERUSER privilege"
    );
    assert!(
        !rolcreaterole,
        "Finding F-04: spine_app MUST NOT possess CREATEROLE privilege"
    );
    assert!(
        !rolcreatedb,
        "Finding F-04: spine_app MUST NOT possess CREATEDB privilege"
    );
    assert!(
        !rolbypassrls,
        "Finding F-04: spine_app MUST NOT possess BYPASSRLS privilege"
    );

    // 2. Attempt unauthorized administrative action: CREATE ROLE (must fail with permission denied error)
    let create_role_res = sqlx::query("CREATE ROLE unauthorized_attacker_role WITH LOGIN")
        .execute(&pool)
        .await;

    assert!(
        create_role_res.is_err(),
        "spine_app must be denied CREATE ROLE execution"
    );
    let err_str = create_role_res.unwrap_err().to_string();
    assert!(
        err_str.contains("permission denied") || err_str.contains("must be superuser"),
        "Error must indicate lack of privilege: {}",
        err_str
    );

    // 3. Attempt unauthorized administrative action: ALTER SYSTEM (must fail)
    let alter_sys_res = sqlx::query("ALTER SYSTEM SET log_connections = 'on'")
        .execute(&pool)
        .await;
    assert!(
        alter_sys_res.is_err(),
        "spine_app must be denied ALTER SYSTEM execution"
    );

    // 4. Verify legitimate application DML queries execute successfully
    let test_user_query = sqlx::query("SELECT count(*) as cnt FROM users")
        .fetch_one(&pool)
        .await;
    assert!(
        test_user_query.is_ok(),
        "spine_app must have SELECT permissions on application tables"
    );
}
