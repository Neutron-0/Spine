use crate::durability::DurabilityPolicy;
use crate::enrollment::{EnrollmentRequest, EnrollmentResponse};
use crate::heartbeat::NodeHeartbeat;
use crate::node::NodeStatus;
use crate::placement::PlacementPlan;
use serde::{Deserialize, Serialize};

/// Hard wire frame bound for cluster RPC messages (64 MiB) to prevent unbounded memory allocation.
pub const MAX_CLUSTER_RPC_FRAME_BYTES: usize = 64 * 1024 * 1024;
/// Maximum permitted compressed snapshot payload per remote participant RPC (15 MiB).
///
/// `ClusterRpcRequest` and `ClusterRpcResponse` are serialized via `serde_json`, where
/// `payload_bytes: Vec<u8>` encodes as a JSON array of decimal integers (`[255,254,...]`).
/// High-entropy bytes (`100..=255`) expand 4x (`"255,"` per byte). At 15 MiB (`15,728,640` bytes),
/// worst-case payload encoding is `62,914,561` bytes, leaving >4 MiB headroom for
/// `VolumeSnapshotManifest` and protocol fields inside `MAX_CLUSTER_RPC_FRAME_BYTES` (64 MiB).
/// Operational consequence: compressed snapshot tar.gz payloads for remote participants sent over
/// single-frame cluster RPC cannot exceed 15 MiB.
pub const MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES: usize = 15 * 1024 * 1024;

const _: () =
    assert!(MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES * 4 + 1024 * 1024 <= MAX_CLUSTER_RPC_FRAME_BYTES);

/// Bounded `std::io::Write` sink backed by `Vec<u8>` that fails immediately before allocating
/// when total written bytes would exceed `max_bytes`.
#[derive(Debug, Clone)]
pub struct BoundedVecWriter {
    buf: Vec<u8>,
    max_bytes: usize,
}

impl BoundedVecWriter {
    pub fn new(max_bytes: usize) -> Self {
        Self {
            buf: Vec::new(),
            max_bytes,
        }
    }

    pub fn into_inner(self) -> Vec<u8> {
        self.buf
    }

    pub fn len(&self) -> usize {
        self.buf.len()
    }

    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

impl std::io::Write for BoundedVecWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let next_len = self.buf.len().saturating_add(data.len());
        if next_len > self.max_bytes {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!(
                    "Remote participant snapshot payload exceeded MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES ({} bytes) during capture; compressed participant volume must not exceed 15 MiB over single-frame RPC",
                    self.max_bytes
                ),
            ));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

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
    QueryStagedVolumeRestore {
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
    #[serde(default)]
    pub restore_id: Option<String>,
    #[serde(default)]
    pub backup_id: Option<String>,
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
    #[serde(default)]
    pub archive_payload_sha256: String,
    #[serde(default)]
    pub staged_resource_digest: String,
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
    StagedVolumeRestoreStatus {
        snapshot_id: String,
        prepared: bool,
        #[serde(default)]
        acknowledgement: Option<ParticipantAcknowledgement>,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_bounded_vec_writer_below_at_and_above_limit() {
        let limit = MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES;

        // Just below limit
        let mut w_below = BoundedVecWriter::new(limit);
        w_below.write_all(&vec![0x11u8; limit - 1]).unwrap();
        assert_eq!(w_below.len(), limit - 1);

        // Exactly at limit
        let mut w_at = BoundedVecWriter::new(limit);
        w_at.write_all(&vec![0x22u8; limit]).unwrap();
        assert_eq!(w_at.len(), limit);

        // Above limit by 1 byte: fails immediately without growing buffer
        let mut w_over = BoundedVecWriter::new(limit);
        w_over.write_all(&vec![0x33u8; limit]).unwrap();
        let err = w_over.write_all(&[0x44u8]).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(w_over.len(), limit);

        // Streaming 100 MiB in 32 KiB chunks stops at 15 MiB + 1 chunk
        let mut w_stream = BoundedVecWriter::new(limit);
        let chunk = vec![0xABu8; 32 * 1024];
        let mut chunks_written = 0usize;
        for _ in 0..(100 * 32) {
            if w_stream.write_all(&chunk).is_err() {
                break;
            }
            chunks_written += 1;
        }
        assert_eq!(chunks_written, limit / (32 * 1024));
        assert_eq!(w_stream.len(), limit);
    }
}
