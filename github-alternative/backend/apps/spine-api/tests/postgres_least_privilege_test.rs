mod common;

use common::HarnessDisposableDb;
use sqlx::Row;

#[tokio::test]
async fn test_spine_app_role_has_no_superuser_privileges() {
    let disp_db = match HarnessDisposableDb::provision().await {
        Ok(db) => db,
        Err(e) => {
            eprintln!("SKIPPED: could not provision disposable database: {}", e);
            return;
        }
    };

    let migrator_pool = disp_db.migrator_pool().await.expect("migrator pool");
    let app_pool = disp_db.app_pool().await.expect("app pool");

    // 1. Verify rolsuper, rolcreaterole, rolcreatedb are FALSE for spine_app in pg_roles
    let role_row = sqlx::query(
        "SELECT rolname, rolsuper, rolcreaterole, rolcreatedb, rolbypassrls FROM pg_roles WHERE rolname = 'spine_app'",
    )
    .fetch_one(&migrator_pool)
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

    // 2. Attempt unauthorized administrative action: CREATE ROLE as spine_app (must fail)
    let create_role_res = sqlx::query("CREATE ROLE unauthorized_attacker_role WITH LOGIN")
        .execute(&app_pool)
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

    // 3. Attempt unauthorized administrative action: CREATE TABLE as spine_app (must fail)
    let create_table_res = sqlx::query("CREATE TABLE unauthorized_test_table (id INT)")
        .execute(&app_pool)
        .await;

    assert!(
        create_table_res.is_err(),
        "spine_app must be denied CREATE TABLE on public schema"
    );
    let table_err_str = create_table_res.unwrap_err().to_string();
    assert!(
        table_err_str.contains("permission denied") || table_err_str.contains("42501"),
        "Error must indicate permission denied for schema public: {}",
        table_err_str
    );

    // 4. Verify legitimate application DML queries execute successfully as spine_app
    let test_user_query = sqlx::query("SELECT count(*) as cnt FROM users")
        .fetch_one(&app_pool)
        .await;
    assert!(
        test_user_query.is_ok(),
        "spine_app must have SELECT permissions on application tables: {:?}",
        test_user_query.err()
    );
}
