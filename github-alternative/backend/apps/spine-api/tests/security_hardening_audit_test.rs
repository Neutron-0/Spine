use spine_api::api::routes_lfs::{
    generate_scoped_token, resolve_lfs_secret_salt, verify_scoped_token,
};
use spine_api::security::service::resolve_fingerprint_secret;
use spine_api::webhook::SsrfValidator;

#[test]
fn test_lfs_secret_salt_configurable_from_env() {
    std::env::set_var("SPINE_LFS_SECRET", "super_secure_production_secret_32b!");
    let salt = resolve_lfs_secret_salt();
    assert_eq!(salt, b"super_secure_production_secret_32b!");

    let (token, _exp) = generate_scoped_token("repo_1", "oid_1", "upload", 1024, 3600);
    let res = verify_scoped_token(&token, "repo_1", "oid_1", "upload");
    assert_eq!(res, Ok(1024));
    std::env::remove_var("SPINE_LFS_SECRET");
}

#[test]
fn test_fingerprint_secret_configurable_from_env() {
    std::env::set_var("SPINE_FINGERPRINT_SECRET", "custom_pepper_salt_9999");
    let salt = resolve_fingerprint_secret();
    assert_eq!(salt, b"custom_pepper_salt_9999");
    std::env::remove_var("SPINE_FINGERPRINT_SECRET");
}

#[tokio::test]
async fn test_ssrf_prohibits_spine_cluster_ports() {
    let validator = SsrfValidator::new(false);
    // Port 8400 (coordinator), 8401 (storage-node), and 2222 (SSH) must be prohibited
    assert!(validator
        .validate_and_resolve("http://example.com:8400/rpc")
        .await
        .is_err());
    assert!(validator
        .validate_and_resolve("http://example.com:8401/storage")
        .await
        .is_err());
    assert!(validator
        .validate_and_resolve("http://example.com:2222/")
        .await
        .is_err());
}
