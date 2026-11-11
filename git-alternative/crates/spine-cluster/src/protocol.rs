use crate::durability::DurabilityPolicy;
use crate::enrollment::{EnrollmentRequest, EnrollmentResponse};
use crate::heartbeat::NodeHeartbeat;
use crate::node::NodeStatus;
use crate::placement::PlacementPlan;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClusterRpcRequest {
    Enroll(EnrollmentRequest),
    Heartbeat(NodeHeartbeat),
    GetPlacement {
        repo_id: String,
    },
    AllocateRepository {
        repo_id: String,
        name: String,
        owner: String,
        durability_policy: DurabilityPolicy,
    },
    DeleteRepository {
        repo_id: String,
    },
    PreparePush {
        transaction_id: String,
        repo_id: String,
        ownership_epoch: u64,
        proposed_generation: u64,
        pack_id: String,
        pack_checksum: String,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    },
    AcknowledgePrepare {
        transaction_id: String,
        node_id: String,
        success: bool,
        error: Option<String>,
    },
    CommitPush {
        transaction_id: String,
        repo_id: String,
        ownership_epoch: u64,
        new_generation: u64,
    },
    AbortPush {
        transaction_id: String,
        repo_id: String,
        ownership_epoch: u64,
        reason: String,
    },
    ReplicatePack {
        repo_id: String,
        pack_id: String,
        pack_checksum: String,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    },
    InspectRepository {
        repo_id: String,
    },
    RepairPack {
        repo_id: String,
        pack_id: String,
        pack_checksum: String,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    },
    DrainNode {
        node_id: String,
    },
    QuarantineNode {
        node_id: String,
    },
    RetireNode {
        node_id: String,
    },
    StoreChunk {
        chunk_hash: String,
        chunk_bytes: Vec<u8>,
    },
    GetChunk {
        chunk_hash: String,
    },
    StoreManifest {
        manifest_id: String,
        manifest_bytes: Vec<u8>,
    },
    GetManifest {
        manifest_id: String,
    },
    EnterMaintenanceBarrier {
        fencing_generation: u64,
    },
    ExitMaintenanceBarrier {
        fencing_generation: u64,
    },
    QueryMaintenanceStatus,
    CaptureVolumeSnapshot {
        snapshot_id: String,
        fencing_generation: u64,
    },
    StageVolumeRestore {
        snapshot_id: String,
        fencing_generation: u64,
        manifest: VolumeSnapshotManifest,
        payload_bytes: Vec<u8>,
    },
    CommitVolumeRestore {
        snapshot_id: String,
        fencing_generation: u64,
    },
    RollbackVolumeRestore {
        snapshot_id: String,
    },
    FinalizeVolumeRestore {
        snapshot_id: String,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct VolumeSnapshotManifest {
    pub participant_id: String,
    pub volume_name: String,
    pub snapshot_id: String,
    pub fencing_generation: u64,
    pub file_count: usize,
    pub total_bytes: u64,
    pub sha256_checksum: String,
    pub created_at_nanos: i64,
}

/// Durable preparation acknowledgement for an authoritative restore participant.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParticipantAcknowledgement {
    pub restore_id: String,
    pub backup_id: String,
    pub participant_id: String,
    pub volume_name: String,
    #[serde(default)]
    pub canonical_resource_id: String,
    pub previous_generation: Option<String>,
    pub target_generation: String,
    pub staged_digest: String,
    pub staged_byte_length: u64,
    pub prepared_at_nanos: i64,
    pub prepared: bool,
    #[serde(default)]
    pub durable_staging_record_id: String,
    #[serde(default)]
    pub current_state: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ClusterRpcResponse {
    EnrollSuccess(EnrollmentResponse),
    HeartbeatAck {
        status: NodeStatus,
    },
    PlacementResult(Option<PlacementPlan>),
    AllocationResult {
        repo_id: String,
        placement: PlacementPlan,
    },
    DeleteResult {
        repo_id: String,
        success: bool,
    },
    PrepareAck {
        transaction_id: String,
        prepared: bool,
    },
    CommitAck {
        transaction_id: String,
        committed_generation: u64,
    },
    AbortAck {
        transaction_id: String,
    },
    ReplicateAck {
        pack_id: String,
        verified: bool,
    },
    InspectResult {
        repo_id: String,
        generation: u64,
        packs: Vec<String>,
        is_healthy: bool,
    },
    RepairAck {
        pack_id: String,
        success: bool,
    },
    StoreChunkAck {
        chunk_hash: String,
        success: bool,
    },
    GetChunkAck {
        chunk_hash: String,
        chunk_bytes: Option<Vec<u8>>,
    },
    StoreManifestAck {
        manifest_id: String,
        success: bool,
    },
    GetManifestAck {
        manifest_id: String,
        manifest_bytes: Option<Vec<u8>>,
    },
    NodeStatusUpdated {
        node_id: String,
        new_status: NodeStatus,
    },
    MaintenanceStatus {
        in_maintenance: bool,
        fencing_generation: u64,
        active_writers: usize,
        #[serde(default)]
        node_id: Option<String>,
    },
    MaintenanceBarrierAck {
        success: bool,
    },
    VolumeSnapshotCaptured {
        manifest: VolumeSnapshotManifest,
        payload_bytes: Vec<u8>,
    },
    VolumeRestoreStaged {
        snapshot_id: String,
        success: bool,
        #[serde(default)]
        acknowledgement: Option<ParticipantAcknowledgement>,
    },
    VolumeRestoreCommitted {
        snapshot_id: String,
        success: bool,
    },
    VolumeRestoreRolledBack {
        snapshot_id: String,
        success: bool,
    },
    VolumeRestoreFinalized {
        snapshot_id: String,
        success: bool,
    },
    Error(String),
}

impl ClusterRpcRequest {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}

impl ClusterRpcResponse {
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        serde_json::from_slice(bytes)
    }
}
