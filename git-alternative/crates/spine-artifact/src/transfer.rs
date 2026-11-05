use serde::{Deserialize, Serialize};
use spine_cluster::durability::DurabilityPolicy;
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransferError {
    #[error("Invalid state transition from {from:?} to {to:?}")]
    InvalidStateTransition {
        from: TransferState,
        to: TransferState,
    },
    #[error("Session {0} expired")]
    SessionExpired(String),
    #[error("Invalid chunk index {index}, expected < {expected_total}")]
    InvalidChunkIndex { index: u32, expected_total: u32 },
    #[error("Missing chunks during finalization: {0:?}")]
    MissingChunks(Vec<u32>),
    #[error("Durability condition unsatisfied: required {required}, verified {actual}")]
    DurabilityUnsatisfied { required: usize, actual: usize },
    #[error("Transfer error: {0}")]
    Other(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferState {
    Created,
    Uploading,
    Verifying,
    Replicating,
    Finalizing,
    Complete,
    Aborted,
    Expired,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferSession {
    pub session_id: String,
    pub artifact_id: String,
    pub state: TransferState,
    pub total_size: u64,
    pub chunk_size: u32,
    pub expected_chunks: u32,
    pub uploaded_chunks: HashSet<u32>,
    pub durability_policy: DurabilityPolicy,
    pub created_at_nanos: i64,
    pub expires_at_nanos: i64,
}

impl TransferSession {
    pub fn new(
        session_id: String,
        artifact_id: String,
        total_size: u64,
        chunk_size: u32,
        durability_policy: DurabilityPolicy,
        ttl_seconds: i64,
    ) -> Self {
        let expected_chunks = if total_size == 0 {
            1
        } else {
            total_size.div_ceil(chunk_size as u64) as u32
        };

        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let expires_at_nanos = now_nanos + (ttl_seconds * 1_000_000_000i64);

        Self {
            session_id,
            artifact_id,
            state: TransferState::Created,
            total_size,
            chunk_size,
            expected_chunks,
            uploaded_chunks: HashSet::new(),
            durability_policy,
            created_at_nanos: now_nanos,
            expires_at_nanos,
        }
    }

    pub fn is_expired(&self, now_nanos: i64) -> bool {
        now_nanos > self.expires_at_nanos
            && self.state != TransferState::Complete
            && self.state != TransferState::Aborted
    }

    pub fn record_chunk_uploaded(&mut self, index: u32) -> Result<(), TransferError> {
        if index >= self.expected_chunks {
            return Err(TransferError::InvalidChunkIndex {
                index,
                expected_total: self.expected_chunks,
            });
        }

        match self.state {
            TransferState::Created => {
                self.state = TransferState::Uploading;
            }
            TransferState::Uploading => {}
            _ => {
                return Err(TransferError::InvalidStateTransition {
                    from: self.state,
                    to: TransferState::Uploading,
                });
            }
        }

        self.uploaded_chunks.insert(index);
        Ok(())
    }

    pub fn is_all_chunks_uploaded(&self) -> bool {
        self.uploaded_chunks.len() == self.expected_chunks as usize
    }

    pub fn missing_chunks(&self) -> Vec<u32> {
        let mut missing = Vec::new();
        for i in 0..self.expected_chunks {
            if !self.uploaded_chunks.contains(&i) {
                missing.push(i);
            }
        }
        missing
    }

    pub fn transition_to(&mut self, next: TransferState) -> Result<(), TransferError> {
        let valid = match (self.state, next) {
            (TransferState::Created, TransferState::Uploading) => true,
            (TransferState::Created, TransferState::Aborted) => true,
            (TransferState::Created, TransferState::Expired) => true,

            (TransferState::Uploading, TransferState::Verifying) => self.is_all_chunks_uploaded(),
            (TransferState::Uploading, TransferState::Aborted) => true,
            (TransferState::Uploading, TransferState::Expired) => true,

            (TransferState::Verifying, TransferState::Replicating) => true,
            (TransferState::Verifying, TransferState::Aborted) => true,

            (TransferState::Replicating, TransferState::Finalizing) => true,
            (TransferState::Replicating, TransferState::Aborted) => true,

            (TransferState::Finalizing, TransferState::Complete) => true,
            (TransferState::Finalizing, TransferState::Aborted) => true,

            _ => false,
        };

        if !valid {
            return Err(TransferError::InvalidStateTransition {
                from: self.state,
                to: next,
            });
        }

        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_lifecycle_state_machine() {
        let mut session = TransferSession::new(
            "sess_1".into(),
            "art_1".into(),
            1000,
            500, // 2 chunks
            DurabilityPolicy::SingleCopy,
            3600,
        );

        assert_eq!(session.expected_chunks, 2);
        assert_eq!(session.state, TransferState::Created);

        // Upload chunk 0
        session.record_chunk_uploaded(0).unwrap();
        assert_eq!(session.state, TransferState::Uploading);
        assert!(!session.is_all_chunks_uploaded());

        // Attempt verify prematurely fails
        assert!(session.transition_to(TransferState::Verifying).is_err());

        // Upload chunk 1
        session.record_chunk_uploaded(1).unwrap();
        assert!(session.is_all_chunks_uploaded());

        // Verify -> Replicating -> Finalizing -> Complete
        session.transition_to(TransferState::Verifying).unwrap();
        session.transition_to(TransferState::Replicating).unwrap();
        session.transition_to(TransferState::Finalizing).unwrap();
        session.transition_to(TransferState::Complete).unwrap();

        assert_eq!(session.state, TransferState::Complete);
    }
}
