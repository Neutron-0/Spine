use crate::core::error::ForgeError;
use crate::core::profile::DeploymentProfile;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Classification of participant requirement under a deployment profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ParticipantClassification {
    Required,
    Optional,
}

/// Canonical machine-readable specification of a persistent deployment resource.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeploymentResource {
    pub participant_id: String,
    pub resource_identity: String,
    pub owner_service: String,
    pub classification: ParticipantClassification,
    pub snapshot_mechanism: String,
    pub restore_mechanism: String,
    pub generation_identity: String,
    pub manifest_schema: String,
    pub auth_mechanism: String,
    pub integrity_validation: String,
    pub readiness_requirements: String,
}

/// Authoritative participant registry for deployment-wide backup and disaster recovery.
pub struct ParticipantRegistry;

impl ParticipantRegistry {
    /// Returns the canonical production participant inventory (all 7 authoritative resources).
    pub fn production_registry() -> Vec<DeploymentResource> {
        vec![
            DeploymentResource {
                participant_id: "postgres_data".to_string(),
                resource_identity: "postgres_data".to_string(),
                owner_service: "postgres".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism:
                    "Streaming JSON Lines dump (version 3.0) via pg_dump / SQL catalog query"
                        .to_string(),
                restore_mechanism:
                    "Transactional JSON Lines import with pre-commit _restore_generation injection"
                        .to_string(),
                generation_identity: "Table public._restore_generation (id=1, generation_token)"
                    .to_string(),
                manifest_schema: "jsonl_snapshot_v3".to_string(),
                auth_mechanism:
                    "PostgreSQL least-privilege SCRAM-SHA-256 (spine_migrator/spine_app)"
                        .to_string(),
                integrity_validation: "Row count validation, JSON schema check, SHA-256 hash match"
                    .to_string(),
                readiness_requirements: "PostgreSQL port 5432 healthy, pg_isready exit 0"
                    .to_string(),
            },
            DeploymentResource {
                participant_id: "spine_storage".to_string(),
                resource_identity: "spine_storage".to_string(),
                owner_service: "spine-api".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism: "Filesystem scan and SHA-256 hashing into tar builder"
                    .to_string(),
                restore_mechanism:
                    "Atomic whole-generation directory replacement with journaled rollback"
                        .to_string(),
                generation_identity: "File .spine_storage_generation in storage root".to_string(),
                manifest_schema: "manifest.json file_hashes BTreeMap".to_string(),
                auth_mechanism: "Local filesystem UID 10001 file ownership".to_string(),
                integrity_validation: "Cryptographic SHA-256 digest match for every entry"
                    .to_string(),
                readiness_requirements: "Storage root mounted read-write, adequate capacity"
                    .to_string(),
            },
            DeploymentResource {
                participant_id: "node-1".to_string(),
                resource_identity: "storage_1_data".to_string(),
                owner_service: "spine-storage-node-1".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism: "RPC CaptureVolumeSnapshot -> in-memory tar.gz stream"
                    .to_string(),
                restore_mechanism:
                    "RPC StageVolumeRestore -> CommitVolumeRestore (retaining .old_restore)"
                        .to_string(),
                generation_identity:
                    "File .spine_storage_generation and persisted .fencing_generation".to_string(),
                manifest_schema: "VolumeSnapshotManifest JSON".to_string(),
                auth_mechanism: "Internal network mutual RPC fencing verification".to_string(),
                integrity_validation: "SHA-256 checksum match and BLAKE3 chunk verification"
                    .to_string(),
                readiness_requirements:
                    "Storage node 1 RPC online, maintenance barrier acknowledged".to_string(),
            },
            DeploymentResource {
                participant_id: "node-2".to_string(),
                resource_identity: "storage_2_data".to_string(),
                owner_service: "spine-storage-node-2".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism: "RPC CaptureVolumeSnapshot -> in-memory tar.gz stream"
                    .to_string(),
                restore_mechanism:
                    "RPC StageVolumeRestore -> CommitVolumeRestore (retaining .old_restore)"
                        .to_string(),
                generation_identity:
                    "File .spine_storage_generation and persisted .fencing_generation".to_string(),
                manifest_schema: "VolumeSnapshotManifest JSON".to_string(),
                auth_mechanism: "Internal network mutual RPC fencing verification".to_string(),
                integrity_validation: "SHA-256 checksum match and BLAKE3 chunk verification"
                    .to_string(),
                readiness_requirements:
                    "Storage node 2 RPC online, maintenance barrier acknowledged".to_string(),
            },
            DeploymentResource {
                participant_id: "node-3".to_string(),
                resource_identity: "storage_3_data".to_string(),
                owner_service: "spine-storage-node-3".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism: "RPC CaptureVolumeSnapshot -> in-memory tar.gz stream"
                    .to_string(),
                restore_mechanism:
                    "RPC StageVolumeRestore -> CommitVolumeRestore (retaining .old_restore)"
                        .to_string(),
                generation_identity:
                    "File .spine_storage_generation and persisted .fencing_generation".to_string(),
                manifest_schema: "VolumeSnapshotManifest JSON".to_string(),
                auth_mechanism: "Internal network mutual RPC fencing verification".to_string(),
                integrity_validation: "SHA-256 checksum match and BLAKE3 chunk verification"
                    .to_string(),
                readiness_requirements:
                    "Storage node 3 RPC online, maintenance barrier acknowledged".to_string(),
            },
            DeploymentResource {
                participant_id: "coordinator".to_string(),
                resource_identity: "coordinator_data".to_string(),
                owner_service: "spine-coordinator".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism:
                    "RPC CaptureVolumeSnapshot -> coordinator StateSnapshot JSON payload"
                        .to_string(),
                restore_mechanism:
                    "RPC StageVolumeRestore -> CommitVolumeRestore with state snapshot rollback"
                        .to_string(),
                generation_identity: "File .coordinator_fencing_generation in state dir"
                    .to_string(),
                manifest_schema: "VolumeSnapshotManifest JSON".to_string(),
                auth_mechanism: "Internal network mutual RPC fencing verification".to_string(),
                integrity_validation:
                    "SHA-256 checksum match and StateSnapshot deserialization check".to_string(),
                readiness_requirements: "Coordinator RPC online, maintenance barrier acknowledged"
                    .to_string(),
            },
            DeploymentResource {
                participant_id: "spine_ssh".to_string(),
                resource_identity: "spine_ssh".to_string(),
                owner_service: "spine-api".to_string(),
                classification: ParticipantClassification::Required,
                snapshot_mechanism:
                    "Tar memory stream of host_key_ed25519 and host_key_ed25519.pub".to_string(),
                restore_mechanism:
                    "Direct unpack and OpenSSH format verification of private/public keys"
                        .to_string(),
                generation_identity: "Top-level backup manifest fencing generation and fingerprint"
                    .to_string(),
                manifest_schema:
                    "VolumeSnapshotManifest JSON + BackupManifest.ssh_host_key_fingerprint"
                        .to_string(),
                auth_mechanism: "Encrypted SPNEBAK v2 AEAD envelope (AES-256-GCM)".to_string(),
                integrity_validation:
                    "SHA-256 checksum match and russh PrivateKey parser validation".to_string(),
                readiness_requirements: "Valid Ed25519 host key pair present in SSH directory"
                    .to_string(),
            },
        ]
    }

    /// Returns canonical volume name for a given participant ID.
    pub fn canonical_volume_for_participant(participant_id: &str) -> Option<&'static str> {
        match participant_id {
            "postgres_data" => Some("postgres_data"),
            "spine_storage" => Some("spine_storage"),
            "node-1" | "storage_1_data" => Some("storage_1_data"),
            "node-2" | "storage_2_data" => Some("storage_2_data"),
            "node-3" | "storage_3_data" => Some("storage_3_data"),
            "coordinator" | "coordinator_data" => Some("coordinator_data"),
            "spine_ssh" => Some("spine_ssh"),
            _ => None,
        }
    }

    /// Validates whether an archive relative path matches the expected path pattern for a participant.
    pub fn is_valid_archive_path_for_participant(participant_id: &str, path: &str) -> bool {
        match participant_id {
            "postgres_data" => {
                path == "database/postgres_data.jsonl"
                    || path == "database/snapshot.jsonl"
                    || path == "database/snapshot.json"
                    || path == "database/catalog_dump.jsonl"
                    || path.starts_with("database/")
            }
            "spine_storage" => {
                path == "storage/spine_storage_snapshot.tar.gz" || path.starts_with("storage/")
            }
            "node-1" | "storage_1_data" => {
                path == "storage_nodes/storage_1_data.tar.gz"
                    || path == "storage_nodes/node-1.tar.gz"
                    || path == "storage_nodes/node-1_snapshot.tar.gz"
            }
            "node-2" | "storage_2_data" => {
                path == "storage_nodes/storage_2_data.tar.gz"
                    || path == "storage_nodes/node-2.tar.gz"
                    || path == "storage_nodes/node-2_snapshot.tar.gz"
            }
            "node-3" | "storage_3_data" => {
                path == "storage_nodes/storage_3_data.tar.gz"
                    || path == "storage_nodes/node-3.tar.gz"
                    || path == "storage_nodes/node-3_snapshot.tar.gz"
            }
            "coordinator" | "coordinator_data" => {
                path == "coordinator/coordinator_data.tar.gz"
                    || path == "coordinator/coordinator_snapshot.tar.gz"
                    || path.starts_with("coordinator/")
            }
            "spine_ssh" => {
                path == "ssh/spine_ssh.tar.gz"
                    || path == "ssh/ssh_identity_snapshot.tar.gz"
                    || path.starts_with("ssh/")
            }
            _ => false,
        }
    }

    /// Verifies that all expected participants for the given deployment profile are present,
    /// complete, uncorrupted, and consistently fenced.
    pub fn validate_participants(
        profile: &DeploymentProfile,
        participants: &[super::BackupParticipantRecord],
        fencing_generation: u64,
    ) -> Result<(), ForgeError> {
        let mut seen_ids = HashSet::new();
        let mut seen_paths = HashSet::new();
        for p in participants {
            if !seen_ids.insert(p.participant_id.as_str()) {
                return Err(ForgeError::Internal(format!(
                    "Duplicate participant ID '{}' rejected in backup participant list",
                    p.participant_id
                )));
            }
            if !seen_paths.insert(p.archive_rel_path.as_str()) {
                return Err(ForgeError::Internal(format!(
                    "Duplicate participant archive path '{}' rejected in backup participant list",
                    p.archive_rel_path
                )));
            }
            let expected_vol = Self::canonical_volume_for_participant(&p.participant_id)
                .ok_or_else(|| {
                    ForgeError::Internal(format!(
                        "Unrecognized participant ID '{}' in backup manifest",
                        p.participant_id
                    ))
                })?;
            if p.volume_name != expected_vol {
                return Err(ForgeError::Internal(format!(
                    "Participant-to-volume substitution rejected: participant '{}' declared volume '{}', expected '{}'",
                    p.participant_id, p.volume_name, expected_vol
                )));
            }
            if !Self::is_valid_archive_path_for_participant(&p.participant_id, &p.archive_rel_path)
            {
                return Err(ForgeError::Internal(format!(
                    "Unexpected archive path '{}' for participant '{}'",
                    p.archive_rel_path, p.participant_id
                )));
            }
            if p.byte_length == 0 {
                return Err(ForgeError::Internal(format!(
                    "Participant '{}' ({}) has 0 bytes: empty participant snapshots are rejected",
                    p.participant_id, p.volume_name
                )));
            }
            if p.sha256_checksum.len() != 64 {
                return Err(ForgeError::Internal(format!(
                    "Participant '{}' has invalid SHA-256 checksum: '{}'",
                    p.participant_id, p.sha256_checksum
                )));
            }
            if p.fencing_generation != fencing_generation {
                return Err(ForgeError::Internal(format!(
                    "Participant '{}' has mismatched fencing generation {} (expected {})",
                    p.participant_id, p.fencing_generation, fencing_generation
                )));
            }
        }

        if profile.requires_persistent_database() {
            let required = Self::production_registry();
            for req in &required {
                let matches_req = seen_ids.contains(req.participant_id.as_str())
                    || (req.participant_id == "node-1" && seen_ids.contains("storage_1_data"))
                    || (req.participant_id == "node-2" && seen_ids.contains("storage_2_data"))
                    || (req.participant_id == "node-3" && seen_ids.contains("storage_3_data"))
                    || (req.participant_id == "coordinator"
                        && seen_ids.contains("coordinator_data"));
                if !matches_req {
                    return Err(ForgeError::Internal(format!(
                        "FATAL BACKUP INCOMPLETE: Under deployment profile '{}', required participant '{}' ({}) is missing from archive! Incomplete cluster backups are strictly forbidden.",
                        profile, req.participant_id, req.resource_identity
                    )));
                }
            }
        }

        Ok(())
    }
}
