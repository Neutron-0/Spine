use serde::{Deserialize, Serialize};
use std::fmt;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NodeError {
    #[error("Invalid node state transition from {from:?} to {to:?}")]
    InvalidStateTransition { from: NodeStatus, to: NodeStatus },
    #[error("Node not found: {0}")]
    NotFound(String),
    #[error("Node is quarantined: {0}")]
    Quarantined(String),
    #[error("Node is offline: {0}")]
    Offline(String),
    #[error("Node is draining: {0}")]
    Draining(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeStatus {
    Joining,
    Healthy,
    Suspect,
    Offline,
    Quarantined,
    Draining,
    Retired,
}

impl fmt::Display for NodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NodeStatus::Joining => write!(f, "JOINING"),
            NodeStatus::Healthy => write!(f, "HEALTHY"),
            NodeStatus::Suspect => write!(f, "SUSPECT"),
            NodeStatus::Offline => write!(f, "OFFLINE"),
            NodeStatus::Quarantined => write!(f, "QUARANTINED"),
            NodeStatus::Draining => write!(f, "DRAINING"),
            NodeStatus::Retired => write!(f, "RETIRED"),
        }
    }
}

impl NodeStatus {
    pub fn can_transition_to(&self, next: NodeStatus) -> bool {
        use NodeStatus::*;
        if *self == next {
            return true;
        }
        match self {
            Joining => matches!(next, Healthy | Quarantined | Offline),
            Healthy => matches!(next, Suspect | Quarantined | Draining | Offline),
            Suspect => matches!(next, Healthy | Offline | Quarantined),
            Offline => matches!(next, Joining | Suspect | Healthy | Quarantined | Retired),
            Quarantined => matches!(next, Joining | Draining | Retired | Offline),
            Draining => matches!(next, Retired | Offline | Healthy | Quarantined),
            Retired => false, // Terminal state
        }
    }

    pub fn is_write_eligible(&self) -> bool {
        matches!(self, NodeStatus::Healthy)
    }

    pub fn is_read_eligible(&self) -> bool {
        matches!(
            self,
            NodeStatus::Healthy | NodeStatus::Suspect | NodeStatus::Draining
        )
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FailureDomain {
    pub region: String,
    pub zone: String,
    pub host: String,
}

impl Default for FailureDomain {
    fn default() -> Self {
        Self {
            region: "local".into(),
            zone: "zone-1".into(),
            host: "localhost".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeCapacity {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub reserved_bytes: u64,
}

impl NodeCapacity {
    #[allow(clippy::too_many_arguments)]
    pub fn new(total_bytes: u64, used_bytes: u64, reserved_bytes: u64) -> Self {
        let available_bytes = total_bytes.saturating_sub(used_bytes + reserved_bytes);
        Self {
            total_bytes,
            used_bytes,
            available_bytes,
            reserved_bytes,
        }
    }

    pub fn can_reserve(&self, bytes: u64) -> bool {
        self.available_bytes >= bytes
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageNodeInfo {
    pub node_id: String,
    pub public_key_hex: String,
    pub fingerprint: String,
    pub status: NodeStatus,
    pub listen_addr: String,
    pub storage_root: String,
    pub failure_domain: FailureDomain,
    pub storage_class: String,
    pub capacity: NodeCapacity,
    pub enrolled_at_nanos: i64,
    pub last_heartbeat_nanos: i64,
    pub active_tasks: u32,
    pub error_count: u32,
}

impl StorageNodeInfo {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        node_id: String,
        public_key_hex: String,
        fingerprint: String,
        listen_addr: String,
        storage_root: String,
        failure_domain: FailureDomain,
        total_capacity_bytes: u64,
        now_nanos: i64,
    ) -> Self {
        Self {
            node_id,
            public_key_hex,
            fingerprint,
            status: NodeStatus::Joining,
            listen_addr,
            storage_root,
            failure_domain,
            storage_class: "standard".into(),
            capacity: NodeCapacity::new(total_capacity_bytes, 0, 0),
            enrolled_at_nanos: now_nanos,
            last_heartbeat_nanos: now_nanos,
            active_tasks: 0,
            error_count: 0,
        }
    }

    pub fn transition_to(&mut self, new_status: NodeStatus) -> Result<(), NodeError> {
        if !self.status.can_transition_to(new_status) {
            return Err(NodeError::InvalidStateTransition {
                from: self.status,
                to: new_status,
            });
        }
        self.status = new_status;
        Ok(())
    }
}
