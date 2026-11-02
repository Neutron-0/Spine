use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DurabilityError {
    #[error("Durability requirement not met: required {required} copies, but only {actual} acknowledged")]
    InsufficientCopies { required: usize, actual: usize },
    #[error("Primary node not prepared")]
    PrimaryNotPrepared,
    #[error("Replica node '{0}' failed verification")]
    ReplicaVerificationFailed(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DurabilityPolicy {
    SingleCopy,
    TwoCopies,
    ThreeCopies,
}

impl fmt::Display for DurabilityPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DurabilityPolicy::SingleCopy => write!(f, "SINGLE_COPY"),
            DurabilityPolicy::TwoCopies => write!(f, "TWO_COPIES"),
            DurabilityPolicy::ThreeCopies => write!(f, "THREE_COPIES"),
        }
    }
}

impl DurabilityPolicy {
    pub fn required_copies(&self) -> usize {
        match self {
            DurabilityPolicy::SingleCopy => 1,
            DurabilityPolicy::TwoCopies => 2,
            DurabilityPolicy::ThreeCopies => 3,
        }
    }

    pub fn required_replicas(&self) -> usize {
        self.required_copies().saturating_sub(1)
    }

    pub fn is_satisfied(&self, verified_copies_count: usize) -> bool {
        verified_copies_count >= self.required_copies()
    }

    pub fn verify_satisfaction(&self, verified_copies_count: usize) -> Result<(), DurabilityError> {
        if self.is_satisfied(verified_copies_count) {
            Ok(())
        } else {
            Err(DurabilityError::InsufficientCopies {
                required: self.required_copies(),
                actual: verified_copies_count,
            })
        }
    }
}
