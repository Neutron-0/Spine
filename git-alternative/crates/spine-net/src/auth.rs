//! Cryptographic challenge-response authentication, asymmetric Ed25519 verification,
//! replay protection cache, and role-based access control (RBAC).

use crate::identity::{parse_verifying_key, verify_signature};
use ed25519_dalek::VerifyingKey;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::Path;
use std::str::FromStr;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

pub const DEFAULT_SECRET_KEY: &[u8; 32] = b"spine-development-auth-secret-32";
const AUTH_TRANSCRIPT_DOMAIN: &[u8] = b"SPWP-AUTH-v1\x00";
pub const MAX_REPLAY_CACHE_ENTRIES: usize = 20_000;
pub const MAX_CHALLENGE_AGE_MS: u64 = 120_000; // 120 seconds
pub const MAX_FUTURE_SKEW_MS: u64 = 60_000; // 60 seconds

#[derive(Error, Debug, PartialEq, Eq)]
pub enum AuthError {
    #[error("Challenge expired: timestamp {0} ms outside valid window")]
    ChallengeExpired(u64),
    #[error("Replay attack detected: nonce has already been used")]
    ReplayDetected,
    #[error("Invalid authentication signature")]
    InvalidSignature,
    #[error("Public key not recognized: {0}")]
    UnrecognizedKey(String),
    #[error("Public key has been revoked: {0}")]
    KeyRevoked(String),
    #[error("Public key credentials have expired")]
    KeyExpired,
    #[error("User '{0}' is not authorized for operation '{1}'")]
    Unauthorized(String, String),
    #[error("Authentication format error: {0}")]
    FormatError(String),
}

/// Role-based access control roles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Read-only access: clone, fetch, advertise refs.
    Reader,
    /// Read-write access: push fast-forward updates to non-protected branches.
    Contributor,
    /// Administrative access: force-push with lease, create/delete branches, push to protected branches.
    Admin,
}

impl Role {
    pub fn can_read(&self) -> bool {
        true
    }

    pub fn can_push(&self) -> bool {
        matches!(self, Role::Contributor | Role::Admin)
    }

    pub fn can_force_push(&self) -> bool {
        matches!(self, Role::Admin)
    }

    pub fn can_modify_protected(&self) -> bool {
        matches!(self, Role::Admin)
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Role::Reader => write!(f, "reader"),
            Role::Contributor => write!(f, "contributor"),
            Role::Admin => write!(f, "admin"),
        }
    }
}

impl FromStr for Role {
    type Err = AuthError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "reader" | "read" => Ok(Role::Reader),
            "contributor" | "write" => Ok(Role::Contributor),
            "admin" | "owner" => Ok(Role::Admin),
            _ => Err(AuthError::FormatError(format!("Unknown role: {}", s))),
        }
    }
}

/// Short-lived signed capability ticket issued by Spine Forge for remote repository operations.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapabilityTicket {
    pub version: u8,
    pub ticket_id: String,
    pub user_id: String,
    pub username: String,
    pub repo_id: String,
    pub native_repo_id: String,
    pub role: Role,
    pub issued_at_nanos: i64,
    pub expires_at_nanos: i64,
    pub issuer: String,
    pub nonce: String,
}

impl CapabilityTicket {
    pub fn new(
        user_id: String,
        username: String,
        repo_id: String,
        native_repo_id: String,
        role: Role,
        validity_duration_nanos: i64,
        now_nanos: i64,
    ) -> Self {
        Self {
            version: 1,
            ticket_id: format!("tkt_{}", uuid::Uuid::new_v4().simple()),
            user_id,
            username,
            repo_id,
            native_repo_id,
            role,
            issued_at_nanos: now_nanos,
            expires_at_nanos: now_nanos + validity_duration_nanos,
            issuer: "spine-forge-api".to_string(),
            nonce: format!("{:016x}", rand::random::<u64>()),
        }
    }

    pub fn encode_and_sign(&self, secret_key: &[u8; 32]) -> String {
        let json_bytes = serde_json::to_vec(self).unwrap_or_default();
        let payload_hex = hex::encode(json_bytes);
        let sig = blake3::keyed_hash(secret_key, payload_hex.as_bytes())
            .to_hex()
            .to_string();
        format!("{}.{}", payload_hex, sig)
    }

    pub fn decode_and_verify(
        ticket_str: &str,
        secret_key: &[u8; 32],
        now_nanos: i64,
    ) -> Result<Self, AuthError> {
        let parts: Vec<&str> = ticket_str.split('.').collect();
        if parts.len() != 2 {
            return Err(AuthError::FormatError(
                "Invalid capability ticket format".into(),
            ));
        }
        let payload_hex = parts[0];
        let expected_sig = parts[1];
        let computed_sig = blake3::keyed_hash(secret_key, payload_hex.as_bytes())
            .to_hex()
            .to_string();
        if computed_sig != expected_sig {
            return Err(AuthError::InvalidSignature);
        }
        let json_bytes = hex::decode(payload_hex)
            .map_err(|e| AuthError::FormatError(format!("Invalid ticket hex: {}", e)))?;
        let ticket: CapabilityTicket = serde_json::from_slice(&json_bytes)
            .map_err(|e| AuthError::FormatError(format!("Invalid ticket JSON: {}", e)))?;
        if now_nanos > ticket.expires_at_nanos {
            return Err(AuthError::KeyExpired);
        }
        Ok(ticket)
    }
}

/// Deterministic authentication transcript for domain-separated signing.
#[derive(Debug, Clone)]
pub struct AuthTranscript {
    pub protocol_version: u16,
    pub server_id: String,
    pub session_id: u32,
    pub nonce: [u8; 32],
    pub timestamp_ms: u64,
    pub repo_path: String,
    pub scope: String,
}

impl AuthTranscript {
    pub fn new(
        protocol_version: u16,
        server_id: String,
        session_id: u32,
        nonce: [u8; 32],
        timestamp_ms: u64,
        repo_path: String,
        scope: String,
    ) -> Self {
        Self {
            protocol_version,
            server_id,
            session_id,
            nonce,
            timestamp_ms,
            repo_path,
            scope,
        }
    }

    /// Computes the 32-byte domain-separated transcript digest using BLAKE3.
    pub fn compute_digest(&self) -> [u8; 32] {
        let mut hasher = blake3::Hasher::new();
        hasher.update(AUTH_TRANSCRIPT_DOMAIN);
        hasher.update(&self.protocol_version.to_le_bytes());
        hasher.update(&(self.server_id.len() as u32).to_le_bytes());
        hasher.update(self.server_id.as_bytes());
        hasher.update(&self.session_id.to_le_bytes());
        hasher.update(&self.nonce);
        hasher.update(&self.timestamp_ms.to_le_bytes());
        hasher.update(&(self.repo_path.len() as u32).to_le_bytes());
        hasher.update(self.repo_path.as_bytes());
        hasher.update(&(self.scope.len() as u32).to_le_bytes());
        hasher.update(self.scope.as_bytes());

        *hasher.finalize().as_bytes()
    }
}

/// Thread-safe bounded replay protection cache.
#[derive(Debug, Clone)]
pub struct BoundedReplayCache {
    inner: Arc<Mutex<BTreeMap<[u8; 32], u64>>>,
}

impl Default for BoundedReplayCache {
    fn default() -> Self {
        Self::new()
    }
}

impl BoundedReplayCache {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    /// Verifies that the challenge timestamp is within the acceptable window
    /// and that the nonce has not been seen before. Returns `Ok(())` and registers the nonce.
    pub fn check_and_register(&self, nonce: &[u8; 32], timestamp_ms: u64) -> Result<(), AuthError> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Check if timestamp is in the future beyond acceptable skew
        if timestamp_ms > now_ms && timestamp_ms - now_ms > MAX_FUTURE_SKEW_MS {
            return Err(AuthError::ChallengeExpired(timestamp_ms));
        }

        // Check if timestamp is older than window
        if now_ms >= timestamp_ms && now_ms - timestamp_ms > MAX_CHALLENGE_AGE_MS {
            return Err(AuthError::ChallengeExpired(timestamp_ms));
        }

        let mut map = self
            .inner
            .lock()
            .map_err(|_| AuthError::FormatError("Replay cache lock poisoned".to_string()))?;

        // Prune expired entries if capacity reached
        if map.len() >= MAX_REPLAY_CACHE_ENTRIES {
            let cutoff = now_ms.saturating_sub(MAX_CHALLENGE_AGE_MS);
            map.retain(|_, &mut ts| ts >= cutoff);
        }

        // Check for replay
        if map.contains_key(nonce) {
            return Err(AuthError::ReplayDetected);
        }

        map.insert(*nonce, timestamp_ms);
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.inner.lock().map(|m| m.len()).unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// An authorized key entry in the server's key registry.
#[derive(Debug, Clone)]
pub struct AuthorizedKey {
    pub principal: String,
    pub verifying_key: VerifyingKey,
    pub role: Role,
    pub is_revoked: bool,
    pub expires_at: Option<u64>,
}

/// Server registry for authenticated public keys and principal authorizations.
#[derive(Debug, Clone, Default)]
pub struct KeyRegistry {
    keys: Arc<RwLock<HashMap<[u8; 32], AuthorizedKey>>>,
}

impl KeyRegistry {
    pub fn new() -> Self {
        Self {
            keys: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Registers or updates an authorized public key.
    pub fn register_key(
        &self,
        principal: &str,
        verifying_key: VerifyingKey,
        role: Role,
        expires_at: Option<u64>,
    ) {
        let mut map = self.keys.write().unwrap();
        let pub_bytes = verifying_key.to_bytes();
        map.insert(
            pub_bytes,
            AuthorizedKey {
                principal: principal.to_string(),
                verifying_key,
                role,
                is_revoked: false,
                expires_at,
            },
        );
    }

    /// Revokes an authorized public key.
    pub fn revoke_key(&self, verifying_key_bytes: &[u8; 32]) -> bool {
        let mut map = self.keys.write().unwrap();
        if let Some(entry) = map.get_mut(verifying_key_bytes) {
            entry.is_revoked = true;
            true
        } else {
            false
        }
    }

    /// Looks up a key and validates revocation and expiration.
    pub fn lookup(&self, verifying_key_bytes: &[u8; 32]) -> Result<AuthorizedKey, AuthError> {
        let map = self.keys.read().unwrap();
        let entry = map.get(verifying_key_bytes).ok_or_else(|| {
            let fp = format!("key_bytes_{}", hex::encode(&verifying_key_bytes[0..8]));
            AuthError::UnrecognizedKey(fp)
        })?;

        if entry.is_revoked {
            return Err(AuthError::KeyRevoked(entry.principal.clone()));
        }

        if let Some(exp) = entry.expires_at {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            if now_ms > exp {
                return Err(AuthError::KeyExpired);
            }
        }

        Ok(entry.clone())
    }

    /// Loads authorized keys from an `authorized_keys` configuration string.
    /// Format per line: `<role> <principal> <hex_public_key>`
    pub fn load_from_text(&self, text: &str) -> Result<usize, AuthError> {
        let mut count = 0;
        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.len() < 3 {
                continue;
            }
            let role = Role::from_str(parts[0])?;
            let principal = parts[1];
            let key_hex = parts[2];
            let key_bytes = hex::decode(key_hex)
                .map_err(|e| AuthError::FormatError(format!("Invalid key hex: {}", e)))?;
            let vk = parse_verifying_key(&key_bytes)
                .map_err(|e| AuthError::FormatError(e.to_string()))?;

            self.register_key(principal, vk, role, None);
            count += 1;
        }
        Ok(count)
    }

    /// Loads authorized keys from a file.
    pub fn load_from_file(&self, path: &Path) -> Result<usize, AuthError> {
        if !path.exists() {
            return Ok(0);
        }
        let content = std::fs::read_to_string(path)
            .map_err(|e| AuthError::FormatError(format!("Cannot read key file: {}", e)))?;
        self.load_from_text(&content)
    }
}

/// Computes a 32-byte HMAC/keyed-hash authentication signature using BLAKE3 (symmetric fallback).
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

/// Verifies client authentication response against symmetric secret key with replay cache.
pub fn verify_symmetric_auth_signature(
    secret_key: &[u8; 32],
    user_id: &str,
    nonce: &[u8; 32],
    timestamp_ms: u64,
    signature: &[u8],
    replay_cache: &BoundedReplayCache,
) -> Result<bool, AuthError> {
    replay_cache.check_and_register(nonce, timestamp_ms)?;

    if signature.len() != 32 {
        return Ok(false);
    }

    let expected = compute_auth_signature(secret_key, user_id, nonce, timestamp_ms);
    let mut match_result = 0u8;
    for (a, b) in expected.iter().zip(signature.iter()) {
        match_result |= a ^ b;
    }

    Ok(match_result == 0)
}

/// Verifies asymmetric Ed25519 authentication response against registered public keys.
pub fn verify_asymmetric_auth(
    transcript: &AuthTranscript,
    public_key_bytes: &[u8],
    signature: &[u8],
    registry: &KeyRegistry,
    replay_cache: &BoundedReplayCache,
) -> Result<AuthorizedKey, AuthError> {
    // 1. Check replay cache for nonce reuse and timestamp expiration
    replay_cache.check_and_register(&transcript.nonce, transcript.timestamp_ms)?;

    // 2. Validate public key length and lookup in registry
    if public_key_bytes.len() != 32 {
        return Err(AuthError::FormatError(
            "Public key must be 32 bytes".to_string(),
        ));
    }
    let mut pk_arr = [0u8; 32];
    pk_arr.copy_from_slice(public_key_bytes);

    let auth_entry = registry.lookup(&pk_arr)?;

    // 3. Compute transcript digest
    let digest = transcript.compute_digest();

    // 4. Verify Ed25519 signature
    let valid = verify_signature(&auth_entry.verifying_key, &digest, signature)
        .map_err(|e| AuthError::FormatError(e.to_string()))?;

    if !valid {
        return Err(AuthError::InvalidSignature);
    }

    Ok(auth_entry)
}

/// Legacy wrapper for backwards compatibility with Batch 04 tests.
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

    let diff = now_ms.abs_diff(timestamp_ms);
    if diff > MAX_CHALLENGE_AGE_MS {
        return Err(AuthError::ChallengeExpired(timestamp_ms));
    }

    if signature.len() != 32 {
        return Ok(false);
    }

    let expected = compute_auth_signature(secret_key, user_id, nonce, timestamp_ms);
    let mut match_result = 0u8;
    for (a, b) in expected.iter().zip(signature.iter()) {
        match_result |= a ^ b;
    }

    Ok(match_result == 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::SpineIdentity;

    #[test]
    fn test_asymmetric_auth_flow_success() {
        let registry = KeyRegistry::new();
        let cache = BoundedReplayCache::new();

        let alice_identity = SpineIdentity::generate();
        registry.register_key(
            "alice",
            alice_identity.verifying_key(),
            Role::Contributor,
            None,
        );

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let transcript = AuthTranscript::new(
            1,
            "spine-daemon-node-01".to_string(),
            1001,
            [7u8; 32],
            now,
            "demo/repo".to_string(),
            "repo:write".to_string(),
        );

        let digest = transcript.compute_digest();
        let sig = alice_identity.sign(&digest);

        let authed = verify_asymmetric_auth(
            &transcript,
            &alice_identity.public_key_bytes(),
            &sig.to_bytes(),
            &registry,
            &cache,
        )
        .unwrap();

        assert_eq!(authed.principal, "alice");
        assert_eq!(authed.role, Role::Contributor);
    }

    #[test]
    fn test_asymmetric_auth_replay_rejection() {
        let registry = KeyRegistry::new();
        let cache = BoundedReplayCache::new();

        let bob_identity = SpineIdentity::generate();
        registry.register_key("bob", bob_identity.verifying_key(), Role::Admin, None);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let transcript = AuthTranscript::new(
            1,
            "spine-daemon-node-01".to_string(),
            1002,
            [8u8; 32],
            now,
            "secure/repo".to_string(),
            "repo:admin".to_string(),
        );

        let digest = transcript.compute_digest();
        let sig = bob_identity.sign(&digest);

        // First verification succeeds
        assert!(verify_asymmetric_auth(
            &transcript,
            &bob_identity.public_key_bytes(),
            &sig.to_bytes(),
            &registry,
            &cache,
        )
        .is_ok());

        // Replay of same nonce fails
        let err = verify_asymmetric_auth(
            &transcript,
            &bob_identity.public_key_bytes(),
            &sig.to_bytes(),
            &registry,
            &cache,
        )
        .unwrap_err();

        assert_eq!(err, AuthError::ReplayDetected);
    }

    #[test]
    fn test_asymmetric_auth_revoked_key_rejection() {
        let registry = KeyRegistry::new();
        let cache = BoundedReplayCache::new();

        let charlie_identity = SpineIdentity::generate();
        registry.register_key(
            "charlie",
            charlie_identity.verifying_key(),
            Role::Reader,
            None,
        );

        // Revoke charlie's key
        registry.revoke_key(&charlie_identity.public_key_bytes());

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let transcript = AuthTranscript::new(
            1,
            "spine-daemon-node-01".to_string(),
            1003,
            [9u8; 32],
            now,
            "some/repo".to_string(),
            "repo:read".to_string(),
        );

        let digest = transcript.compute_digest();
        let sig = charlie_identity.sign(&digest);

        let err = verify_asymmetric_auth(
            &transcript,
            &charlie_identity.public_key_bytes(),
            &sig.to_bytes(),
            &registry,
            &cache,
        )
        .unwrap_err();

        match err {
            AuthError::KeyRevoked(p) => assert_eq!(p, "charlie"),
            other => panic!("Expected KeyRevoked, got {:?}", other),
        }
    }
}
