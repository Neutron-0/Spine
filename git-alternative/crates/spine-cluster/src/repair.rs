use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepairStatus {
    Queued,
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepairJob {
    pub job_id: String,
    pub repo_id: String,
    pub target_node_id: String,
    pub source_node_id: String,
    pub missing_packs: Vec<String>,
    pub status: RepairStatus,
    pub bytes_transferred: u64,
    pub created_at_nanos: i64,
    pub completed_at_nanos: Option<i64>,
    pub error_message: Option<String>,
}

impl RepairJob {
    pub fn new(
        job_id: String,
        repo_id: String,
        target_node_id: String,
        source_node_id: String,
        missing_packs: Vec<String>,
        now_nanos: i64,
    ) -> Self {
        Self {
            job_id,
            repo_id,
            target_node_id,
            source_node_id,
            missing_packs,
            status: RepairStatus::Queued,
            bytes_transferred: 0,
            created_at_nanos: now_nanos,
            completed_at_nanos: None,
            error_message: None,
        }
    }
}

pub struct AntiEntropy;

impl AntiEntropy {
    /// Compares the pack inventory of a source node against a target replica and returns missing pack IDs.
    pub fn diff_packs(source_packs: &[String], target_packs: &[String]) -> Vec<String> {
        let target_set: HashSet<&String> = target_packs.iter().collect();
        source_packs
            .iter()
            .filter(|p| !target_set.contains(p))
            .cloned()
            .collect()
    }
}
