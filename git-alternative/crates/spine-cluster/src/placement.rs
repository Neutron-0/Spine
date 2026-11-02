use crate::durability::DurabilityPolicy;
use crate::node::StorageNodeInfo;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PlacementError {
    #[error("Insufficient healthy nodes to satisfy replication policy: required {required}, available {available}")]
    InsufficientNodes { required: usize, available: usize },
    #[error("Insufficient storage capacity: required {required_bytes} bytes, maximum available {available_bytes} bytes")]
    InsufficientCapacity {
        required_bytes: u64,
        available_bytes: u64,
    },
    #[error("Repository '{0}' already placed")]
    AlreadyPlaced(String),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlacementPlan {
    pub repo_id: String,
    pub primary_node_id: String,
    pub replica_node_ids: Vec<String>,
    pub required_copies: usize,
    pub placement_generation: u64,
}

impl PlacementPlan {
    pub fn all_nodes(&self) -> Vec<String> {
        let mut nodes = vec![self.primary_node_id.clone()];
        nodes.extend(self.replica_node_ids.clone());
        nodes
    }

    pub fn contains_node(&self, node_id: &str) -> bool {
        self.primary_node_id == node_id || self.replica_node_ids.iter().any(|r| r == node_id)
    }
}

pub struct PlacementScheduler;

impl PlacementScheduler {
    pub fn schedule_placement(
        repo_id: &str,
        policy: DurabilityPolicy,
        available_nodes: &[StorageNodeInfo],
        estimated_repo_size_bytes: u64,
        current_generation: u64,
    ) -> Result<PlacementPlan, PlacementError> {
        let required = policy.required_copies();

        // 1. Filter healthy nodes with sufficient available capacity
        let mut eligible: Vec<&StorageNodeInfo> = available_nodes
            .iter()
            .filter(|n| n.status.is_write_eligible())
            .filter(|n| n.capacity.can_reserve(estimated_repo_size_bytes))
            .collect();

        if eligible.len() < required {
            return Err(PlacementError::InsufficientNodes {
                required,
                available: eligible.len(),
            });
        }

        // 2. Sort eligible nodes by available capacity descending, active tasks ascending
        eligible.sort_by(|a, b| {
            b.capacity
                .available_bytes
                .cmp(&a.capacity.available_bytes)
                .then_with(|| a.active_tasks.cmp(&b.active_tasks))
        });

        // 3. Greedily select nodes maximizing failure domain diversity
        let mut selected: Vec<&StorageNodeInfo> = Vec::new();
        let mut used_hosts = HashSet::new();
        let mut used_zones = HashSet::new();

        // First pass: select across distinct hosts & zones if possible
        for node in &eligible {
            if selected.len() == required {
                break;
            }
            let host = &node.failure_domain.host;
            let zone = &node.failure_domain.zone;
            if !used_hosts.contains(host) && !used_zones.contains(zone) {
                selected.push(node);
                used_hosts.insert(host);
                used_zones.insert(zone);
            }
        }

        // Second pass: fill remaining from distinct hosts
        if selected.len() < required {
            for node in &eligible {
                if selected.len() == required {
                    break;
                }
                if selected.iter().any(|s| s.node_id == node.node_id) {
                    continue;
                }
                let host = &node.failure_domain.host;
                if !used_hosts.contains(host) {
                    selected.push(node);
                    used_hosts.insert(host);
                }
            }
        }

        // Third pass: fallback to any eligible node if failure domain count is constrained
        if selected.len() < required {
            for node in &eligible {
                if selected.len() == required {
                    break;
                }
                if !selected.iter().any(|s| s.node_id == node.node_id) {
                    selected.push(node);
                }
            }
        }

        if selected.len() < required {
            return Err(PlacementError::InsufficientNodes {
                required,
                available: selected.len(),
            });
        }

        let primary_node_id = selected[0].node_id.clone();
        let replica_node_ids = selected[1..required]
            .iter()
            .map(|n| n.node_id.clone())
            .collect();

        Ok(PlacementPlan {
            repo_id: repo_id.to_string(),
            primary_node_id,
            replica_node_ids,
            required_copies: required,
            placement_generation: current_generation + 1,
        })
    }
}
