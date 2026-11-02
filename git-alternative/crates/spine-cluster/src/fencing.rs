use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum FencingError {
    #[error("Stale ownership epoch: request epoch {request_epoch} is less than current authoritative epoch {current_epoch}")]
    StaleEpoch {
        request_epoch: u64,
        current_epoch: u64,
    },
    #[error("Ownership epoch mismatch: request epoch {request_epoch} does not match current epoch {current_epoch}")]
    EpochMismatch {
        request_epoch: u64,
        current_epoch: u64,
    },
    #[error("Node '{node_id}' is not the designated primary for repository '{repo_id}'")]
    NotPrimary { node_id: String, repo_id: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct OwnershipEpoch(pub u64);

impl OwnershipEpoch {
    pub fn new(epoch: u64) -> Self {
        Self(epoch)
    }

    pub fn initial() -> Self {
        Self(1)
    }

    pub fn next(&self) -> Self {
        Self(self.0 + 1)
    }

    pub fn value(&self) -> u64 {
        self.0
    }

    pub fn validate(&self, request_epoch: u64) -> Result<(), FencingError> {
        if request_epoch < self.0 {
            Err(FencingError::StaleEpoch {
                request_epoch,
                current_epoch: self.0,
            })
        } else if request_epoch > self.0 {
            Err(FencingError::EpochMismatch {
                request_epoch,
                current_epoch: self.0,
            })
        } else {
            Ok(())
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FencingToken {
    pub repo_id: String,
    pub epoch: u64,
    pub primary_node_id: String,
}

impl FencingToken {
    pub fn new(repo_id: String, epoch: u64, primary_node_id: String) -> Self {
        Self {
            repo_id,
            epoch,
            primary_node_id,
        }
    }
}
