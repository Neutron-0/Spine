use spine_api::api::routes_lfs::{
    generate_remote_auth_token, resolve_lfs_secret_salt, verify_remote_auth_token,
};
use spine_api::auth::validate_production_secrets;
use spine_api::DeploymentProfile;

#[test]
fn test_static_salt_cannot_forge_remote_auth_token() {
    // Attempting to forge a token using the legacy static salt string:
    let legacy_static_salt = b"spine-lfs-transfer-token-salt-v1";
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let forged_token = generate_remote_auth_token(
        legacy_static_salt,
        "admin-user-id",
        "target-repo-id",
        "upload",
        now + 3600 * 1_000_000_000,
    );

    // The server's active salt (either random ephemeral or configured production secret)
    let server_salt = resolve_lfs_secret_salt();

    // Verifying with server's salt must reject the forged token
    let verified = verify_remote_auth_token(&server_salt, &forged_token);
    assert!(
        verified.is_none(),
        "Forged token created with static legacy salt must NOT be verified by server"
    );
}

#[test]
fn test_validate_production_secrets_enforcement() {
    // When secrets are unset or empty, CoreForge must fail validation
    std::env::remove_var("SPINE_AUTH_SECRET_KEY");
    std::env::remove_var("SPINE_LFS_SECRET");

    let res = validate_production_secrets(DeploymentProfile::CoreForge);
    assert!(
        res.is_err(),
        "CoreForge must reject startup when secrets are missing"
    );

    // When secret is too short (< 32 chars)
    std::env::set_var("SPINE_AUTH_SECRET_KEY", "too-short-secret");
    let res = validate_production_secrets(DeploymentProfile::CoreForge);
    assert!(
        res.is_err(),
        "CoreForge must reject startup when secret is less than 32 characters"
    );

    // When secret is >= 32 chars
    std::env::set_var(
        "SPINE_AUTH_SECRET_KEY",
        "a-very-long-and-secure-secret-key-with-sufficient-entropy-for-production",
    );
    let res = validate_production_secrets(DeploymentProfile::CoreForge);
    assert!(
        res.is_ok(),
        "CoreForge must accept startup when secret is at least 32 characters"
    );

    // Development profile should allow empty secret
    std::env::remove_var("SPINE_AUTH_SECRET_KEY");
    let res_dev = validate_production_secrets(DeploymentProfile::Development);
    assert!(
        res_dev.is_ok(),
        "Development profile should not mandate production secrets"
    );
}
