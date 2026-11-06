use url::Url;

/// Safely retrieve and validate the isolated test database URL from environment.
///
/// Non-negotiable safety rules:
/// 1. Only reads `SPINE_TEST_DATABASE_URL` or `TEST_DATABASE_URL`.
/// 2. Never falls back to `DATABASE_URL` or `spine_forge`.
/// 3. Validates that the database name contains "test" and is NOT "spine_forge", "postgres", or "template*".
/// 4. Returns `None` if the variable is not set, allowing tests to skip safely without corrupting development data.
pub fn get_safe_test_db_url() -> Option<String> {
    let raw_url = std::env::var("SPINE_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("TEST_DATABASE_URL"))
        .ok()?;

    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return None;
    }

    let parsed = match Url::parse(trimmed) {
        Ok(u) => u,
        Err(e) => panic!("Malformed SPINE_TEST_DATABASE_URL: {}", e),
    };

    let db_name = parsed
        .path_segments()
        .and_then(|mut segs| segs.next())
        .unwrap_or("")
        .trim();

    if db_name.is_empty() {
        panic!("SAFETY ABORT: SPINE_TEST_DATABASE_URL does not specify a database name.");
    }

    if db_name == "spine_forge" || db_name == "postgres" || db_name.starts_with("template") {
        panic!(
            "SAFETY ABORT: Refusing to run tests against protected database '{}'. Tests must run against an isolated test database containing 'test' (e.g. 'spine_forge_test').",
            db_name
        );
    }

    if !db_name.contains("test") {
        panic!(
            "SAFETY ABORT: Database name '{}' does not contain 'test'. Tests must run against a disposable test database.",
            db_name
        );
    }

    Some(trimmed.to_string())
}
