use crate::node::{NodeCapacity, NodeStatus};
use serde::{Deserialize, Serialize};

pub const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 2;
pub const DEFAULT_SUSPECT_TIMEOUT_NANOS: i64 = 6_000_000_000; // 6s
pub const DEFAULT_OFFLINE_TIMEOUT_NANOS: i64 = 15_000_000_000; // 15s

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeHeartbeat {
    pub node_id: String,
    pub timestamp_nanos: i64,
    pub node_generation: u64,
    pub capacity: NodeCapacity,
    pub active_repos: u32,
    pub active_replication_tasks: u32,
    pub error_counter: u32,
    pub status: NodeStatus,
    pub protocol_version: String,
}

impl NodeHeartbeat {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_id: String,
        timestamp_nanos: i64,
        node_generation: u64,
        capacity: NodeCapacity,
        active_repos: u32,
        active_replication_tasks: u32,
        error_counter: u32,
        status: NodeStatus,
    ) -> Self {
        Self {
            node_id,
            timestamp_nanos,
            node_generation,
            capacity,
            active_repos,
            active_replication_tasks,
            error_counter,
            status,
            protocol_version: "1.0.0".into(),
        }
    }
}

pub fn evaluate_node_health(
    last_heartbeat_nanos: i64,
    now_nanos: i64,
    current_status: NodeStatus,
    suspect_threshold_nanos: i64,
    offline_threshold_nanos: i64,
) -> NodeStatus {
    if current_status == NodeStatus::Quarantined
        || current_status == NodeStatus::Draining
        || current_status == NodeStatus::Retired
    {
        return current_status;
    }

    let elapsed = now_nanos.saturating_sub(last_heartbeat_nanos);

    if elapsed >= offline_threshold_nanos {
        NodeStatus::Offline
    } else if elapsed >= suspect_threshold_nanos {
        NodeStatus::Suspect
    } else {
        NodeStatus::Healthy
    }
}
