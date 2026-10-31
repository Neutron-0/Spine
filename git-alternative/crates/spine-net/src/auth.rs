//! Challenge-response authentication and authorization for Spine Wire Protocol.

use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const DEFAULT_SECRET_KEY: &[u8; 32] = b"spine-development-auth-secret-32";

#[derive(Error, Debug, PartialEq, Eq)]
pub enum AuthError {
    #[error("Challenge expired: timestamp {0} ms outside valid window")]
    ChallengeExpired(u64),
    #[error("Invalid authentication signature")]
    InvalidSignature,
    #[error("User '{0}' is not authorized for operation '{1}'")]
    Unauthorized(String, String),
}

/// Computes a 32-byte HMAC/keyed-hash authentication signature using BLAKE3.
pub fn compute_auth_signature(
    secret_key: &[u8; 32],
    user_id: &str,
    nonce: &[u8; 32],
    timestamp_ms: u64,
) -> [u8; 32] {
    let mut payload = Vec::with_capacity(32 + 8 + user_id.len());
    payload.extend_from_slice(nonce);
    payload.extend_from_slice(&timestamp_ms.to_le_bytes());
    payload.extend_from_slice(user_id.as_bytes());

    let hash = blake3::keyed_hash(secret_key, &payload);
    *hash.as_bytes()
}

/// Verifies client authentication response against the server's secret key.
pub fn verify_auth_signature(
    secret_key: &[u8; 32],
    user_id: &str,
    nonce: &[u8; 32],
    timestamp_ms: u64,
    signature: &[u8],
) -> Result<bool, AuthError> {
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    // Reject challenges older than 120 seconds or more than 60 seconds in the future
    let diff = now_ms.abs_diff(timestamp_ms);

    if diff > 120_000 {
        return Err(AuthError::ChallengeExpired(timestamp_ms));
    }

    if signature.len() != 32 {
        return Ok(false);
    }

    let expected = compute_auth_signature(secret_key, user_id, nonce, timestamp_ms);
    let mut expected_sig = [0u8; 32];
    expected_sig.copy_from_slice(&expected);

    // Constant-time comparison
    let mut match_result = 0u8;
    for (a, b) in expected_sig.iter().zip(signature.iter()) {
        match_result |= a ^ b;
    }

    Ok(match_result == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_auth_signature_verification() {
        let secret = [42u8; 32];
        let nonce = [7u8; 32];
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let sig = compute_auth_signature(&secret, "alice", &nonce, now);
        let valid = verify_auth_signature(&secret, "alice", &nonce, now, &sig).unwrap();
        assert!(valid);

        // Tampered user fails
        let invalid_user = verify_auth_signature(&secret, "bob", &nonce, now, &sig).unwrap();
        assert!(!invalid_user);

        // Tampered nonce fails
        let mut tampered_nonce = nonce;
        tampered_nonce[0] ^= 0xFF;
        let invalid_nonce =
            verify_auth_signature(&secret, "alice", &tampered_nonce, now, &sig).unwrap();
        assert!(!invalid_nonce);

        // Expired timestamp fails
        let expired_ts = now - 200_000;
        let err = verify_auth_signature(&secret, "alice", &nonce, expired_ts, &sig).unwrap_err();
        match err {
            AuthError::ChallengeExpired(_) => {}
            _ => panic!("Expected ChallengeExpired"),
        }
    }
}
