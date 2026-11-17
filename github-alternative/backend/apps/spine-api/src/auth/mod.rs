pub mod mail;
pub mod password;
pub mod pat;
pub mod rate_limit;
pub mod session;

pub use mail::{generate_reset_token, generate_verification_token, DevMailCatcher, MailDelivery};
pub use password::{hash_password, validate_password_strength, verify_password};
pub use pat::{generate_pat, has_scope, hash_pat};
pub use rate_limit::RateLimiter;
pub use session::{generate_session_token, hash_session_token, DEFAULT_SESSION_TTL_NANOS};

/// Validates that required security secrets are configured when running under production profiles.
pub fn validate_production_secrets(profile: crate::DeploymentProfile) -> Result<(), anyhow::Error> {
    if profile.requires_persistent_database() || profile == crate::DeploymentProfile::CoreForge {
        let secret = std::env::var("SPINE_AUTH_SECRET_KEY")
            .or_else(|_| std::env::var("SPINE_LFS_SECRET"))
            .unwrap_or_default();
        let trimmed = secret.trim();
        if trimmed.is_empty() {
            anyhow::bail!(
                "CRITICAL SECURITY CONFIGURATION ERROR: Under deployment profile '{}', a secure secret key (SPINE_AUTH_SECRET_KEY or SPINE_LFS_SECRET) of at least 32 characters is required. Running with an unconfigured or empty secret is strictly forbidden in production.",
                profile
            );
        }
        if trimmed.len() < 32 {
            anyhow::bail!(
                "CRITICAL SECURITY CONFIGURATION ERROR: Under deployment profile '{}', the configured secret key (SPINE_AUTH_SECRET_KEY or SPINE_LFS_SECRET) must be at least 32 characters long (got {}).",
                profile,
                trimmed.len()
            );
        }
    }
    Ok(())
}
