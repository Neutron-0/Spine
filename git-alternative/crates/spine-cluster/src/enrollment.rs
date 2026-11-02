use ed25519_dalek::{Signature, VerifyingKey};
use rand::rngs::OsRng;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum EnrollmentError {
    #[error("Enrollment token expired")]
    TokenExpired,
    #[error("Enrollment token already used")]
    TokenAlreadyUsed,
    #[error("Enrollment token revoked")]
    TokenRevoked,
    #[error("Enrollment token not found")]
    TokenNotFound,
    #[error("Invalid Ed25519 public key: {0}")]
    InvalidPublicKey(String),
    #[error("Invalid signature: {0}")]
    InvalidSignature(String),
    #[error("Cluster ID mismatch")]
    ClusterMismatch,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnrollmentToken {
    pub token: String,
    pub cluster_id: String,
    pub expires_at_nanos: i64,
    pub used: bool,
    pub revoked: bool,
    pub created_at_nanos: i64,
}

impl EnrollmentToken {
    pub fn generate(cluster_id: &str, valid_duration_secs: i64, now_nanos: i64) -> Self {
        let mut bytes = [0u8; 32];
        OsRng.fill_bytes(&mut bytes);
        let token = format!("sptk_{}", hex::encode(bytes));
        let expires_at_nanos = now_nanos + (valid_duration_secs * 1_000_000_000);

        Self {
            token,
            cluster_id: cluster_id.to_string(),
            expires_at_nanos,
            used: false,
            revoked: false,
            created_at_nanos: now_nanos,
        }
    }

    pub fn validate(&self, cluster_id: &str, now_nanos: i64) -> Result<(), EnrollmentError> {
        if self.revoked {
            return Err(EnrollmentError::TokenRevoked);
        }
        if self.used {
            return Err(EnrollmentError::TokenAlreadyUsed);
        }
        if now_nanos > self.expires_at_nanos {
            return Err(EnrollmentError::TokenExpired);
        }
        if self.cluster_id != cluster_id {
            return Err(EnrollmentError::ClusterMismatch);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnrollmentRequest {
    pub token: String,
    pub public_key_hex: String,
    pub listen_addr: String,
    pub storage_root: String,
    pub region: String,
    pub zone: String,
    pub host: String,
    pub total_capacity_bytes: u64,
    /// Ed25519 signature over (token + cluster_id + public_key_hex)
    pub signature_hex: String,
}

impl EnrollmentRequest {
    pub fn verify_signature(&self, cluster_id: &str) -> Result<(), EnrollmentError> {
        let pk_bytes = hex::decode(&self.public_key_hex)
            .map_err(|e| EnrollmentError::InvalidPublicKey(e.to_string()))?;
        if pk_bytes.len() != 32 {
            return Err(EnrollmentError::InvalidPublicKey(
                "Public key must be exactly 32 bytes".into(),
            ));
        }
        let mut key_arr = [0u8; 32];
        key_arr.copy_from_slice(&pk_bytes);
        let verifying_key = VerifyingKey::from_bytes(&key_arr)
            .map_err(|e| EnrollmentError::InvalidPublicKey(e.to_string()))?;

        let sig_bytes = hex::decode(&self.signature_hex)
            .map_err(|e| EnrollmentError::InvalidSignature(e.to_string()))?;
        let sig = Signature::from_slice(&sig_bytes)
            .map_err(|e| EnrollmentError::InvalidSignature(e.to_string()))?;

        let message = format!("{}:{}:{}", self.token, cluster_id, self.public_key_hex);
        verifying_key
            .verify_strict(message.as_bytes(), &sig)
            .map_err(|e| EnrollmentError::InvalidSignature(e.to_string()))?;

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EnrollmentResponse {
    pub node_id: String,
    pub cluster_id: String,
    pub enrolled_at_nanos: i64,
}
