use spine_api::db::PgDatabase;

#[test]
fn test_pool_options_custom_env_values() {
    std::env::set_var("DATABASE_MAX_CONNECTIONS", "75");
    std::env::set_var("DATABASE_MIN_CONNECTIONS", "12");
    std::env::set_var("DATABASE_ACQUIRE_TIMEOUT_SECS", "25");

    let settings = PgDatabase::pool_settings_from_env();
    assert_eq!(settings.max_connections, 75);
    assert_eq!(settings.min_connections, 12);
    assert_eq!(settings.acquire_timeout_secs, 25);
    assert_eq!(settings.idle_timeout_secs, 300);
    assert_eq!(settings.max_lifetime_secs, 1800);

    // Clean up
    std::env::remove_var("DATABASE_MAX_CONNECTIONS");
    std::env::remove_var("DATABASE_MIN_CONNECTIONS");
    std::env::remove_var("DATABASE_ACQUIRE_TIMEOUT_SECS");

    let default_settings = PgDatabase::pool_settings_from_env();
    assert_eq!(default_settings.max_connections, 50);
    assert_eq!(default_settings.min_connections, 5);
    assert_eq!(default_settings.acquire_timeout_secs, 10);
}
