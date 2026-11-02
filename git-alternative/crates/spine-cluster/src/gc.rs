use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TombstoneStatus {
    Pending,
    Authorized,
    Deleted,
    Revoked,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TombstoneRecord {
    pub tombstone_id: String,
    pub repo_id: String,
    pub object_id: String,
    pub pack_id: Option<String>,
    pub authorization_generation: u64,
    pub status: TombstoneStatus,
    pub created_at_nanos: i64,
    pub retention_expires_at_nanos: i64,
    pub deleted_at_nanos: Option<i64>,
}

impl TombstoneRecord {
    pub fn new(
        tombstone_id: String,
        repo_id: String,
        object_id: String,
        pack_id: Option<String>,
        authorization_generation: u64,
        retention_duration_secs: i64,
        now_nanos: i64,
    ) -> Self {
        let retention_expires_at_nanos = now_nanos + (retention_duration_secs * 1_000_000_000);
        Self {
            tombstone_id,
            repo_id,
            object_id,
            pack_id,
            authorization_generation,
            status: TombstoneStatus::Pending,
            created_at_nanos: now_nanos,
            retention_expires_at_nanos,
            deleted_at_nanos: None,
        }
    }

    pub fn is_eligible_for_physical_deletion(
        &self,
        current_generation: u64,
        now_nanos: i64,
    ) -> bool {
        self.status == TombstoneStatus::Authorized
            && current_generation >= self.authorization_generation
            && now_nanos >= self.retention_expires_at_nanos
    }
}

pub struct DistributedGcCoordinator;

impl DistributedGcCoordinator {
    pub fn create_tombstone(
        tombstone_id: String,
        repo_id: String,
        object_id: String,
        pack_id: Option<String>,
        authorization_generation: u64,
        retention_secs: i64,
        now_nanos: i64,
    ) -> TombstoneRecord {
        TombstoneRecord::new(
            tombstone_id,
            repo_id,
            object_id,
            pack_id,
            authorization_generation,
            retention_secs,
            now_nanos,
        )
    }

    pub fn authorize_tombstone(tombstone: &mut TombstoneRecord) {
        tombstone.status = TombstoneStatus::Authorized;
    }
}
