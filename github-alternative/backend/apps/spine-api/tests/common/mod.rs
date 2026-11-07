use url::Url;

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

fn require_safe_test_db_url_internal() -> Result<String, String> {
    let raw_url = match std::env::var("SPINE_TEST_DATABASE_URL") {
        Ok(val) => val,
        Err(_) => {
            return Err(
                "FATAL TEST SAFETY ERROR: 'SPINE_TEST_DATABASE_URL' is not set.\n\
                 Database integration tests require an explicitly provisioned, isolated disposable database\n\
                 (e.g. postgres://spine_app:password@127.0.0.1:5432/spine_forge_test?sslmode=disable).\n\
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
