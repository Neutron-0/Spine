use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const RUNNER_PROTOCOL_VERSION: u32 = 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerRegistrationRequest {
    pub name: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub labels: Vec<String>,
    pub max_concurrency: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerRegistrationResponse {
    pub runner_id: String,
    pub runner_token: String,
    pub registered_at_nanos: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerHeartbeatRequest {
    pub runner_id: String,
    pub active_jobs: u32,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RunnerHeartbeatResponse {
    pub status: String, // "ACTIVE", "DRAINED", "REVOKED"
    pub lease_renewed: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobAcquisitionRequest {
    pub runner_id: String,
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobAssignment {
    pub job_id: String,
    pub run_id: String,
    pub repo_id: String,
    pub commit_oid: String,
    pub ref_name: String,
    pub attempt: u32,
    pub fencing_token: u64,
    pub lease_expires_at_nanos: i64,
    pub timeout_seconds: u64,
    pub job_name: String,
    pub steps: Vec<AssignedStep>,
    pub env: HashMap<String, String>,
    pub secrets: HashMap<String, String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssignedStep {
    pub step_index: u32,
    pub name: String,
    pub command: Option<String>,
    pub uses: Option<String>,
    pub with: HashMap<String, String>,
    pub working_directory: Option<String>,
    pub env: HashMap<String, String>,
    pub timeout_seconds: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LogChunkMessage {
    pub job_id: String,
    pub attempt: u32,
    pub step_index: u32,
    pub stream: String, // "stdout" or "stderr"
    pub timestamp_nanos: i64,
    pub content: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct JobCompletionReport {
    pub job_id: String,
    pub attempt: u32,
    pub fencing_token: u64,
    pub exit_code: i32,
    pub status: String, // "SUCCEEDED", "FAILED", "CANCELLED", "TIMED_OUT"
    pub error_message: Option<String>,
}
