//! Cryptographic peer identity and Ed25519 key management for Spine Wire Protocol.

use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand::rngs::OsRng;
use std::fs;
use std::io;
use std::path::Path;
use thiserror::Error;

pub const KEY_FINGERPRINT_PREFIX: &str = "spkey1";
const KEY_HASH_DOMAIN: &[u8] = b"SPWP-KEY-v1\x00";

#[derive(Error, Debug)]
pub enum IdentityError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid key length: expected 32 bytes, got {0}")]
    InvalidKeyLength(usize),
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),
    #[error("Invalid verifying key bytes: {0}")]
    InvalidVerifyingKey(String),
}

/// A cryptographic peer identity containing an Ed25519 signing key and its corresponding verifying key.
#[derive(Clone)]
pub struct SpineIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

impl SpineIdentity {
    /// Generates a new random Ed25519 keypair using the operating system's CSPRNG (`OsRng`).
    pub fn generate() -> Self {
        let mut csprng = OsRng;
        let signing_key = SigningKey::generate(&mut csprng);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Constructs an identity from 32 raw secret key seed bytes.
    pub fn from_bytes(bytes: &[u8; 32]) -> Self {
        let signing_key = SigningKey::from_bytes(bytes);
        let verifying_key = signing_key.verifying_key();
        Self {
            signing_key,
            verifying_key,
        }
    }

    /// Returns the raw 32-byte secret seed for this identity.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Returns the public verifying key.
    pub fn verifying_key(&self) -> VerifyingKey {
        self.verifying_key
    }

    /// Returns the raw 32-byte public key representation.
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Returns the deterministic fingerprint of the public key:
    /// `spkey1<truncated_blake3_hex>`.
    pub fn fingerprint(&self) -> String {
        fingerprint_from_verifying_key(&self.verifying_key)
    }

    /// Signs an arbitrary message slice using Ed25519.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Loads a secret signing key from a file on disk.
    pub fn load_from_file(path: &Path) -> Result<Self, IdentityError> {
        let bytes = fs::read(path)?;
        if bytes.len() == 32 {
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&bytes);
            Ok(Self::from_bytes(&arr))
        } else if bytes.len() == 64 {
            // Hex-encoded 32-byte key
            let hex_str = std::str::from_utf8(&bytes)
                .map_err(|e| IdentityError::InvalidVerifyingKey(e.to_string()))?;
            let decoded = hex::decode(hex_str.trim())
                .map_err(|e| IdentityError::InvalidVerifyingKey(e.to_string()))?;
            if decoded.len() != 32 {
                return Err(IdentityError::InvalidKeyLength(decoded.len()));
            }
            let mut arr = [0u8; 32];
            arr.copy_from_slice(&decoded);
            Ok(Self::from_bytes(&arr))
        } else {
            Err(IdentityError::InvalidKeyLength(bytes.len()))
        }
    }

    /// Atomically persists this identity's secret key to disk with restricted permissions.
    pub fn save_to_file(&self, path: &Path) -> Result<(), IdentityError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = self.to_bytes();
        spine_core::atomic_write_file(path, &bytes)?;
        Ok(())
    }
}

/// Computes the deterministic public key fingerprint for any Ed25519 `VerifyingKey`.
pub fn fingerprint_from_verifying_key(vk: &VerifyingKey) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(KEY_HASH_DOMAIN);
    hasher.update(&vk.to_bytes());
    let hash = hasher.finalize();
    let hex_part = &hash.to_hex()[..32];
    format!("{}:{}", KEY_FINGERPRINT_PREFIX, hex_part)
}

/// Parses a 32-byte raw public key into an Ed25519 `VerifyingKey`.
pub fn parse_verifying_key(bytes: &[u8]) -> Result<VerifyingKey, IdentityError> {
    if bytes.len() != 32 {
        return Err(IdentityError::InvalidKeyLength(bytes.len()));
    }
    let mut arr = [0u8; 32];
    arr.copy_from_slice(bytes);
    VerifyingKey::from_bytes(&arr).map_err(|e| IdentityError::InvalidVerifyingKey(e.to_string()))
}

/// Verifies an Ed25519 signature over a message using a `VerifyingKey`.
pub fn verify_signature(
    verifying_key: &VerifyingKey,
    message: &[u8],
    signature_bytes: &[u8],
) -> Result<bool, IdentityError> {
    if signature_bytes.len() != 64 {
        return Err(IdentityError::InvalidSignature(format!(
            "Expected 64-byte Ed25519 signature, got {}",
            signature_bytes.len()
        )));
    }
    let mut sig_arr = [0u8; 64];
    sig_arr.copy_from_slice(signature_bytes);
    let sig = Signature::from_bytes(&sig_arr);
    Ok(verifying_key.verify(message, &sig).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_identity_generation_and_signing() {
        let id = SpineIdentity::generate();
        let msg = b"SPWP authenticated packet payload";
        let sig = id.sign(msg);

        let valid = verify_signature(&id.verifying_key(), msg, &sig.to_bytes()).unwrap();
        assert!(valid);

        let invalid = verify_signature(&id.verifying_key(), b"tampered", &sig.to_bytes()).unwrap();
        assert!(!invalid);
    }

    #[test]
    fn test_identity_file_roundtrip() {
        let temp = tempdir().unwrap();
        let key_path = temp.path().join("id_ed25519");

        let id = SpineIdentity::generate();
        id.save_to_file(&key_path).unwrap();

        let loaded = SpineIdentity::load_from_file(&key_path).unwrap();
        assert_eq!(id.to_bytes(), loaded.to_bytes());
        assert_eq!(id.fingerprint(), loaded.fingerprint());
    }

    #[test]
    fn test_fingerprint_prefix() {
        let id = SpineIdentity::generate();
        let fp = id.fingerprint();
        assert!(fp.starts_with("spkey1:"));
        assert_eq!(fp.len(), 7 + 32);
    }
}
