use crate::fencing::OwnershipEpoch;
use crate::node::{NodeStatus, StorageNodeInfo};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum FailoverError {
    #[error("No eligible replica found for repository '{repo_id}': cannot failover safely")]
    NoEligibleReplica { repo_id: String },
    #[error("Current primary '{0}' is still healthy: failover not required")]
    PrimaryStillHealthy(String),
    #[error("Candidate replica '{replica_id}' is stale: has generation {replica_gen}, but committed generation is {committed_gen}")]
    StaleReplica {
        replica_id: String,
        replica_gen: u64,
        committed_gen: u64,
    },
    #[error("Candidate replica '{0}' is not verified")]
    UnverifiedReplica(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReplicaState {
    pub node_id: String,
    pub is_primary: bool,
    pub last_synced_generation: u64,
    pub verified: bool,
    pub last_verified_at_nanos: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromotionResult {
    pub repo_id: String,
    pub old_primary_id: String,
    pub new_primary_id: String,
    pub new_ownership_epoch: u64,
    pub committed_generation: u64,
}

pub struct FailoverCoordinator;

impl FailoverCoordinator {
    pub fn evaluate_and_promote(
        repo_id: &str,
        old_primary_id: &str,
        current_epoch: OwnershipEpoch,
        committed_generation: u64,
        replicas: &[ReplicaState],
        nodes: &[StorageNodeInfo],
    ) -> Result<PromotionResult, FailoverError> {
        // 1. Find all eligible replicas
        let mut candidates = Vec::new();

        for rep in replicas {
            if rep.node_id == old_primary_id {
                continue;
            }

            // Check generation match
            if rep.last_synced_generation != committed_generation {
                continue;
            }

            // Check verified status
            if !rep.verified {
                continue;
            }

            // Check node health in cluster
            if let Some(node_info) = nodes.iter().find(|n| n.node_id == rep.node_id) {
                if node_info.status == NodeStatus::Healthy {
                    candidates.push((rep, node_info));
                }
            }
        }

        if candidates.is_empty() {
            return Err(FailoverError::NoEligibleReplica {
                repo_id: repo_id.to_string(),
            });
        }

        // 2. Select best candidate: lowest active tasks, highest available capacity
        candidates.sort_by(|a, b| {
            a.1.active_tasks.cmp(&b.1.active_tasks).then_with(|| {
                b.1.capacity
                    .available_bytes
                    .cmp(&a.1.capacity.available_bytes)
            })
        });

        let selected = candidates[0].0;
        let new_epoch = current_epoch.next();

        Ok(PromotionResult {
            repo_id: repo_id.to_string(),
            old_primary_id: old_primary_id.to_string(),
            new_primary_id: selected.node_id.clone(),
            new_ownership_epoch: new_epoch.value(),
            committed_generation,
        })
    }
}
