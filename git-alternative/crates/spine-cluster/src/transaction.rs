use crate::durability::DurabilityPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TransactionError {
    #[error("Invalid transaction state transition from {from:?} to {to:?}")]
    InvalidTransition {
        from: TransactionPhase,
        to: TransactionPhase,
    },
    #[error("Transaction already decided: cannot change phase after CommitDecided or Committed")]
    AlreadyDecided,
    #[error(
        "Durability requirement not met to decide commit: required {required}, prepared {actual}"
    )]
    CannotCommitUnderprepared { required: usize, actual: usize },
    #[error("Transaction not found: {0}")]
    NotFound(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionPhase {
    Initiated,
    Preparing,
    Prepared,
    CommitDecided,
    Committed,
    Aborted,
    RecoveryRequired,
}

impl fmt::Display for TransactionPhase {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransactionPhase::Initiated => write!(f, "INITIATED"),
            TransactionPhase::Preparing => write!(f, "PREPARING"),
            TransactionPhase::Prepared => write!(f, "PREPARED"),
            TransactionPhase::CommitDecided => write!(f, "COMMIT_DECIDED"),
            TransactionPhase::Committed => write!(f, "COMMITTED"),
            TransactionPhase::Aborted => write!(f, "ABORTED"),
            TransactionPhase::RecoveryRequired => write!(f, "RECOVERY_REQUIRED"),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicationTransaction {
    pub transaction_id: String,
    pub repo_id: String,
    pub previous_generation: u64,
    pub proposed_generation: u64,
    pub primary_node_id: String,
    pub ownership_epoch: u64,
    pub phase: TransactionPhase,
    pub policy: DurabilityPolicy,
    pub participating_nodes: Vec<String>,
    pub prepared_nodes: HashSet<String>,
    pub commit_decided_at_nanos: Option<i64>,
    pub completed_at_nanos: Option<i64>,
    pub error_message: Option<String>,
}

impl ReplicationTransaction {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        transaction_id: String,
        repo_id: String,
        previous_generation: u64,
        proposed_generation: u64,
        primary_node_id: String,
        ownership_epoch: u64,
        policy: DurabilityPolicy,
        participating_nodes: Vec<String>,
    ) -> Self {
        Self {
            transaction_id,
            repo_id,
            previous_generation,
            proposed_generation,
            primary_node_id,
            ownership_epoch,
            phase: TransactionPhase::Initiated,
            policy,
            participating_nodes,
            prepared_nodes: HashSet::new(),
            commit_decided_at_nanos: None,
            completed_at_nanos: None,
            error_message: None,
        }
    }

    pub fn start_preparing(&mut self) -> Result<(), TransactionError> {
        if self.phase != TransactionPhase::Initiated {
            return Err(TransactionError::InvalidTransition {
                from: self.phase,
                to: TransactionPhase::Preparing,
            });
        }
        self.phase = TransactionPhase::Preparing;
        Ok(())
    }

    pub fn acknowledge_prepared(&mut self, node_id: &str) -> Result<bool, TransactionError> {
        if self.phase != TransactionPhase::Preparing && self.phase != TransactionPhase::Prepared {
            return Err(TransactionError::InvalidTransition {
                from: self.phase,
                to: TransactionPhase::Prepared,
            });
        }

        if self.participating_nodes.iter().any(|n| n == node_id) {
            self.prepared_nodes.insert(node_id.to_string());
        }

        if self.prepared_nodes.len() >= self.policy.required_copies() {
            self.phase = TransactionPhase::Prepared;
            Ok(true) // Policy satisfied
        } else {
            Ok(false)
        }
    }

    pub fn decide_commit(&mut self, now_nanos: i64) -> Result<(), TransactionError> {
        if self.prepared_nodes.len() < self.policy.required_copies() {
            return Err(TransactionError::CannotCommitUnderprepared {
                required: self.policy.required_copies(),
                actual: self.prepared_nodes.len(),
            });
        }
        if self.phase != TransactionPhase::Prepared {
            return Err(TransactionError::InvalidTransition {
                from: self.phase,
                to: TransactionPhase::CommitDecided,
            });
        }
        self.phase = TransactionPhase::CommitDecided;
        self.commit_decided_at_nanos = Some(now_nanos);
        Ok(())
    }

    pub fn mark_committed(&mut self, now_nanos: i64) -> Result<(), TransactionError> {
        if self.phase != TransactionPhase::CommitDecided {
            return Err(TransactionError::InvalidTransition {
                from: self.phase,
                to: TransactionPhase::Committed,
            });
        }
        self.phase = TransactionPhase::Committed;
        self.completed_at_nanos = Some(now_nanos);
        Ok(())
    }

    pub fn abort(&mut self, reason: &str, now_nanos: i64) -> Result<(), TransactionError> {
        if self.phase == TransactionPhase::CommitDecided
            || self.phase == TransactionPhase::Committed
        {
            return Err(TransactionError::AlreadyDecided);
        }
        self.phase = TransactionPhase::Aborted;
        self.completed_at_nanos = Some(now_nanos);
        self.error_message = Some(reason.to_string());
        Ok(())
    }

    pub fn mark_recovery_required(&mut self, reason: &str) {
        self.phase = TransactionPhase::RecoveryRequired;
        self.error_message = Some(reason.to_string());
    }

    pub fn recover_on_restart(&mut self, now_nanos: i64) -> TransactionPhase {
        match self.phase {
            TransactionPhase::CommitDecided => {
                // Must complete committed decision
                self.phase = TransactionPhase::Committed;
                self.completed_at_nanos = Some(now_nanos);
                TransactionPhase::Committed
            }
            TransactionPhase::Initiated | TransactionPhase::Preparing => {
                // Incomplete write was never decided -> safely abort
                self.phase = TransactionPhase::Aborted;
                self.completed_at_nanos = Some(now_nanos);
                self.error_message = Some("Aborted during coordinator crash recovery".into());
                TransactionPhase::Aborted
            }
            _ => self.phase,
        }
    }
}
