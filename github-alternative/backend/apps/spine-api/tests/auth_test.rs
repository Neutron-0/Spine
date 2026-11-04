use spine_api::auth::{
    generate_pat, generate_session_token, has_scope, hash_password, hash_pat, hash_session_token,
    validate_password_strength, verify_password, RateLimiter,
};
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn test_password_strength_validation() {
    // Too short (< 8 chars)
    assert!(validate_password_strength("Short1!").is_err());

    // Missing uppercase
    assert!(validate_password_strength("alllower123!").is_err());

    // Missing lowercase
    assert!(validate_password_strength("ALLUPPER123!").is_err());

    // Missing digits
    assert!(validate_password_strength("NoDigitsHere!").is_err());

    // Valid strong passwords
    assert!(validate_password_strength("P@ssw0rd2026_Secure").is_ok());
    assert!(validate_password_strength("SpineVCS_Forge_2026!").is_ok());
    assert!(validate_password_strength("RustDistributed123#").is_ok());
}

#[tokio::test]
async fn test_argon2id_password_hashing() {
    let password = "SuperSecretPassword123!";
    let hash = hash_password(password).expect("Hashing should succeed");

    assert!(hash.starts_with("$argon2id$"));
    assert!(verify_password(password, &hash).expect("Verification should succeed"));
    assert!(!verify_password("WrongPassword123!", &hash).unwrap_or(false));
}

#[tokio::test]
async fn test_session_token_generation_and_hashing() {
    let raw_token = generate_session_token();
    assert!(raw_token.starts_with("sp_sess_"));
    assert_eq!(raw_token.len(), 8 + 64); // "sp_sess_" + 64 hex chars

    let hash1 = hash_session_token(&raw_token);
    let hash2 = hash_session_token(&raw_token);
    assert_eq!(hash1, hash2); // Deterministic BLAKE3 hash
}

#[tokio::test]
async fn test_pat_generation_and_verification() {
    let raw_pat = generate_pat();
    assert!(raw_pat.starts_with("sp_pat_"));

    let hash = hash_pat(&raw_pat);

    let pat = PersonalAccessToken {
        id: "pat_test_1".to_string(),
        user_id: "user_test_1".to_string(),
        name: "CLI Token".to_string(),
        token_hash: hash,
        scopes: vec!["repo:read".to_string(), "repo:write".to_string()],
        expires_at_nanos: 0,
        created_at_nanos: 100,
        last_used_at_nanos: None,
        revoked: false,
    };

    assert!(has_scope(&pat.scopes, "repo:read"));
    assert!(has_scope(&pat.scopes, "repo:write"));
    assert!(!has_scope(&pat.scopes, "admin:all"));
}

#[tokio::test]
async fn test_rate_limiter_sliding_window() {
    let limiter = RateLimiter::new();

    // First 3 requests must be allowed
    assert!(limiter.check_and_record("client_ip_1", 3, 1));
    assert!(limiter.check_and_record("client_ip_1", 3, 1));
    assert!(limiter.check_and_record("client_ip_1", 3, 1));

    // 4th request within window must be rejected
    assert!(!limiter.check_and_record("client_ip_1", 3, 1));

    // Different IP must still be allowed
    assert!(limiter.check_and_record("client_ip_2", 3, 1));

    // Wait for window to slide
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(limiter.check_and_record("client_ip_1", 3, 1));
}

#[tokio::test]
async fn test_database_user_lifecycle() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());

    let user = User {
        id: "usr_01".to_string(),
        username: "developer_one".to_string(),
        normalized_username: "developer_one".to_string(),
        email: "dev1@spine.build".to_string(),
        normalized_email: "dev1@spine.build".to_string(),
        display_name: "Developer One".to_string(),
        password_hash: hash_password("SpineStrongPass1!").unwrap(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1000,
        updated_at_nanos: 1000,
        last_login_nanos: None,
    };

    db.create_user(user.clone())
        .await
        .expect("User creation succeeds");

    // Retrieve by ID, username, and email
    let by_id = db
        .get_user_by_id("usr_01")
        .await
        .unwrap()
        .expect("Found by ID");
    assert_eq!(by_id.username, "developer_one");

    let by_name = db
        .get_user_by_username("developer_one")
        .await
        .unwrap()
        .expect("Found by name");
    assert_eq!(by_name.email, "dev1@spine.build");

    let by_email = db
        .get_user_by_email("dev1@spine.build")
        .await
        .unwrap()
        .expect("Found by email");
    assert_eq!(by_email.id, "usr_01");

    // Verify duplicate username or email fails
    let dup_user = User {
        id: "usr_02".to_string(),
        username: "developer_one".to_string(),
        normalized_username: "developer_one".to_string(),
        email: "other@spine.build".to_string(),
        normalized_email: "other@spine.build".to_string(),
        display_name: "Developer Two".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1001,
        updated_at_nanos: 1001,
        last_login_nanos: None,
    };
    assert!(db.create_user(dup_user).await.is_err());

    // Update password
    let new_hash = hash_password("SpineNewPass2026!").unwrap();
    db.update_user_password("usr_01", &new_hash, 2000)
        .await
        .expect("Password update succeeds");

    let updated = db.get_user_by_id("usr_01").await.unwrap().unwrap();
    assert_eq!(updated.password_hash, new_hash);
}

#[tokio::test]
async fn test_database_session_and_token_revocation() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());

    let raw_token = generate_session_token();
    let token_h = hash_session_token(&raw_token);
    let session = UserSession {
        id: "sess_01".to_string(),
        user_id: "usr_01".to_string(),
        session_token_hash: token_h.clone(),
        ip_address: Some("127.0.0.1".to_string()),
        user_agent: Some("spine-cli/0.1.0".to_string()),
        created_at_nanos: 100,
        expires_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) + 1_000_000_000,
        revoked: false,
    };

    db.create_session(session).await.expect("Session created");

    let found = db
        .get_session_by_hash(&token_h)
        .await
        .unwrap()
        .expect("Session found");
    assert_eq!(found.id, "sess_01");
    assert!(!found.revoked);

    // Revoke session
    db.revoke_session("sess_01").await.expect("Session revoked");
    let after_revoke = db
        .get_session_by_hash(&token_h)
        .await
        .unwrap()
        .expect("Session still exists");
    assert!(after_revoke.revoked);

    // Revoke all sessions for user
    db.revoke_all_user_sessions("usr_01")
        .await
        .expect("All user sessions revoked");
    let sessions = db.list_user_sessions("usr_01").await.unwrap();
    for s in sessions {
        assert!(s.revoked);
    }
}
