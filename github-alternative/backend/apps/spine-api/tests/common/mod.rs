#![allow(dead_code)]

use sqlx::PgPool;
use url::Url;

pub const DISPOSABLE_MARKER_TABLE: &str = "_spine_disposable_test_marker";
pub const DISPOSABLE_MARKER_ID: &str = "disposable_harness_v1";
pub const DISPOSABLE_CONFIRMATION: &str =
    "I_ACKNOWLEDGE_THIS_DATABASE_IS_DISPOSABLE_AND_MAY_BE_CLEANED";

/// Safely retrieve and validate the isolated test database URL from environment.
///
/// Non-negotiable safety rules:
/// 1. Only reads `SPINE_TEST_DATABASE_URL`.
/// 2. Never falls back to `DATABASE_URL` or `spine_forge`.
/// 3. Validates that the database name contains "test" and is NOT "spine_forge", "postgres", or "template*".
/// 4. Fails hard if `SPINE_TEST_DATABASE_URL` is missing or invalid, preventing silent false-positive test passes.
#[allow(dead_code)]
pub fn get_safe_test_db_url() -> Option<String> {
    match require_safe_test_db_url_internal() {
        Ok(url) => Some(url),
        Err(e) => panic!("{}", e),
    }
}

pub fn require_safe_test_db_url() -> String {
    match require_safe_test_db_url_internal() {
        Ok(url) => url,
        Err(e) => panic!("{}", e),
    }
}

#[allow(dead_code)]
pub fn get_app_role_test_db_url() -> String {
    let mut parsed = Url::parse(&require_safe_test_db_url()).expect("valid test db url");
    let _ = parsed.set_username("spine_app");
    parsed.to_string()
}

/// Verifies that the connected database has an explicit disposable marker proving it was created
/// or designated as a disposable test harness database.
///
/// Refuses to proceed with destructive operations against any database lacking this marker,
/// even if its name contains "test".
pub async fn verify_disposable_database(pool: &PgPool) -> Result<(), String> {
    // 1. Verify destructive test opt-in is enabled in environment
    let allow_destructive = std::env::var("SPINE_ALLOW_DESTRUCTIVE_TESTS")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    if !allow_destructive {
        return Err(
            "SAFETY ABORT: 'SPINE_ALLOW_DESTRUCTIVE_TESTS=1' must be explicitly set in the environment \
             to execute destructive database integration tests."
                .to_string(),
        );
    }

    // 2. Check if the disposable marker table exists
    let table_exists: (bool,) = sqlx::query_as(
        r#"
        SELECT EXISTS (
            SELECT 1 FROM information_schema.tables 
            WHERE table_schema = 'public' AND table_name = $1
        )
        "#,
    )
    .bind(DISPOSABLE_MARKER_TABLE)
    .fetch_one(pool)
    .await
    .map_err(|e| format!("Failed to check for disposable test marker table: {}", e))?;

    if !table_exists.0 {
        return Err(format!(
            "SAFETY ABORT: Refusing destructive operation. Table '{}' is absent. \
             The test harness requires an explicit disposable marker to verify the target database \
             is a disposable test instance and not a persistent database.",
            DISPOSABLE_MARKER_TABLE
        ));
    }

    // 3. Verify confirmation token inside the marker table
    let query_sql = format!(
        "SELECT confirmation FROM \"{}\" WHERE marker_id = $1",
        DISPOSABLE_MARKER_TABLE
    );
    let marker_row: Option<(String,)> = sqlx::query_as(&query_sql)
        .bind(DISPOSABLE_MARKER_ID)
        .fetch_optional(pool)
        .await
        .map_err(|e| format!("Failed to read disposable marker record: {}", e))?;

    match marker_row {
        Some((confirmation,)) if confirmation == DISPOSABLE_CONFIRMATION => Ok(()),
        Some((other,)) => Err(format!(
            "SAFETY ABORT: Invalid confirmation token '{}' in disposable marker. Expected '{}'.",
            other, DISPOSABLE_CONFIRMATION
        )),
        None => Err(format!(
            "SAFETY ABORT: Marker record '{}' not found in '{}'. Database is not certified disposable.",
            DISPOSABLE_MARKER_ID, DISPOSABLE_MARKER_TABLE
        )),
    }
}

/// Explicitly provisions the disposable test marker in the target test database.
/// This should only be invoked during test harness setup when creating or initializing a test database.
pub async fn ensure_disposable_marker(pool: &PgPool) -> Result<(), String> {
    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let create_sql = format!(
        r#"
        CREATE TABLE IF NOT EXISTS "{}" (
            marker_id VARCHAR(64) PRIMARY KEY,
            harness_version VARCHAR(32) NOT NULL,
            created_at_nanos BIGINT NOT NULL,
            confirmation VARCHAR(255) NOT NULL
        )
        "#,
        DISPOSABLE_MARKER_TABLE
    );

    sqlx::query(&create_sql)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to create disposable test marker table: {}", e))?;

    let insert_sql = format!(
        r#"
        INSERT INTO "{}" (marker_id, harness_version, created_at_nanos, confirmation)
        VALUES ($1, '21c', $2, $3)
        ON CONFLICT (marker_id) DO UPDATE SET 
            created_at_nanos = $2,
            confirmation = $3
        "#,
        DISPOSABLE_MARKER_TABLE
    );

    sqlx::query(&insert_sql)
        .bind(DISPOSABLE_MARKER_ID)
        .bind(now_nanos)
        .bind(DISPOSABLE_CONFIRMATION)
        .execute(pool)
        .await
        .map_err(|e| format!("Failed to write disposable test marker record: {}", e))?;

    Ok(())
}

fn require_safe_test_db_url_internal() -> Result<String, String> {
    let raw_url = match std::env::var("SPINE_TEST_DATABASE_URL") {
        Ok(val) => val,
        Err(_) => {
            return Err(
                "FATAL TEST SAFETY ERROR: 'SPINE_TEST_DATABASE_URL' is not set.\n\
                 Database integration tests require an explicitly provisioned, isolated disposable database\n\
                 (e.g. postgres://spine_migrator:password@127.0.0.1:5432/spine_forge_test?sslmode=disable).\n\
                 Refusing to silently skip or pass without executing the real database path."
                    .to_string(),
            );
        }
    };

    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(
            "FATAL TEST SAFETY ERROR: 'SPINE_TEST_DATABASE_URL' is set but empty.".to_string(),
        );
    }

    let parsed =
        Url::parse(trimmed).map_err(|e| format!("Malformed SPINE_TEST_DATABASE_URL: {}", e))?;

    let db_name = parsed
        .path_segments()
        .and_then(|mut segs| segs.next())
        .unwrap_or("")
        .trim();

    if db_name.is_empty() {
        return Err(
            "SAFETY ABORT: SPINE_TEST_DATABASE_URL does not specify a database name.".to_string(),
        );
    }

    if db_name == "spine_forge" || db_name == "postgres" || db_name.starts_with("template") {
        return Err(format!(
            "SAFETY ABORT: Refusing to run tests against protected database '{}'. \
             Tests must run against an isolated disposable database containing 'test' (e.g. 'spine_forge_test').",
            db_name
        ));
    }

    if !db_name.contains("test") {
        return Err(format!(
            "SAFETY ABORT: Database name '{}' does not contain 'test'. Tests must run against a disposable test database.",
            db_name
        ));
    }

    Ok(trimmed.to_string())
}
