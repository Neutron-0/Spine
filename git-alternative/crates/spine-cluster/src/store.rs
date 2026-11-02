use crate::durability::DurabilityPolicy;
use crate::enrollment::EnrollmentToken;
use crate::failover::ReplicaState;
use crate::fencing::OwnershipEpoch;
use crate::gc::TombstoneRecord;
use crate::heartbeat::NodeHeartbeat;
use crate::node::{NodeStatus, StorageNodeInfo};
use crate::placement::PlacementPlan;
use crate::repair::RepairJob;
use crate::transaction::ReplicationTransaction;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RepositoryMeta {
    pub repo_id: String,
    pub name: String,
    pub owner: String,
    pub durability_policy: DurabilityPolicy,
    pub committed_generation: u64,
    pub current_ownership_epoch: u64,
    pub storage_quota_bytes: u64,
    pub state: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct StateSnapshot {
    nodes: HashMap<String, StorageNodeInfo>,
    tokens: HashMap<String, EnrollmentToken>,
    repositories: HashMap<String, RepositoryMeta>,
    placements: HashMap<String, PlacementPlan>,
    replicas: HashMap<String, Vec<ReplicaState>>,
    transactions: HashMap<String, ReplicationTransaction>,
    repair_jobs: HashMap<String, RepairJob>,
    tombstones: HashMap<String, TombstoneRecord>,
}

pub trait CoordinatorStore: Send + Sync {
    fn register_node(&self, node: StorageNodeInfo) -> Result<()>;
    fn get_node(&self, node_id: &str) -> Result<Option<StorageNodeInfo>>;
    fn list_nodes(&self) -> Result<Vec<StorageNodeInfo>>;
    fn update_node_status(&self, node_id: &str, status: NodeStatus) -> Result<()>;
    fn record_heartbeat(&self, heartbeat: NodeHeartbeat) -> Result<()>;

    fn create_token(&self, token: EnrollmentToken) -> Result<()>;
    fn get_token(&self, token_str: &str) -> Result<Option<EnrollmentToken>>;
    fn consume_token(&self, token_str: &str) -> Result<()>;
    fn revoke_token(&self, token_str: &str) -> Result<()>;

    fn create_repository(&self, repo: RepositoryMeta) -> Result<()>;
    fn get_repository(&self, repo_id: &str) -> Result<Option<RepositoryMeta>>;
    fn list_repositories(&self) -> Result<Vec<RepositoryMeta>>;
    fn set_placement(&self, placement: PlacementPlan) -> Result<()>;
    fn get_placement(&self, repo_id: &str) -> Result<Option<PlacementPlan>>;

    fn get_replicas(&self, repo_id: &str) -> Result<Vec<ReplicaState>>;
    fn update_replica_state(
        &self,
        repo_id: &str,
        node_id: &str,
        is_primary: bool,
        last_synced_generation: u64,
        verified: bool,
        now_nanos: i64,
    ) -> Result<()>;

    fn get_ownership_epoch(&self, repo_id: &str) -> Result<Option<OwnershipEpoch>>;
    fn update_ownership_epoch(
        &self,
        repo_id: &str,
        epoch: u64,
        primary_node_id: &str,
    ) -> Result<()>;
    fn advance_committed_generation(&self, repo_id: &str, new_gen: u64) -> Result<()>;

    fn create_transaction(&self, tx: ReplicationTransaction) -> Result<()>;
    fn get_transaction(&self, tx_id: &str) -> Result<Option<ReplicationTransaction>>;
    fn update_transaction(&self, tx: ReplicationTransaction) -> Result<()>;
    fn list_active_transactions(&self) -> Result<Vec<ReplicationTransaction>>;

    fn create_repair_job(&self, job: RepairJob) -> Result<()>;
    fn list_repair_jobs(&self) -> Result<Vec<RepairJob>>;

    fn create_tombstone(&self, tombstone: TombstoneRecord) -> Result<()>;
    fn list_tombstones(&self) -> Result<Vec<TombstoneRecord>>;
}

pub struct FileBackedCoordinatorStore {
    path: Option<PathBuf>,
    state: RwLock<StateSnapshot>,
}

impl FileBackedCoordinatorStore {
    pub fn new_in_memory() -> Self {
        Self {
            path: None,
            state: RwLock::new(StateSnapshot::default()),
        }
    }

    pub fn new_file_backed<P: AsRef<Path>>(path: P) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let state = if p.exists() {
            let bytes = fs::read(&p).with_context(|| format!("Reading {}", p.display()))?;
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            StateSnapshot::default()
        };

        Ok(Self {
            path: Some(p),
            state: RwLock::new(state),
        })
    }

    fn persist(&self) -> Result<()> {
        if let Some(ref p) = self.path {
            let snap = self.state.read().unwrap();
            let json = serde_json::to_vec_pretty(&*snap)?;
            let tmp = p.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            fs::write(&tmp, json)?;
            if fs::rename(&tmp, p).is_err() {
                let _ = fs::remove_file(p);
                fs::rename(&tmp, p)?;
            }
        }
        Ok(())
    }
}

impl CoordinatorStore for FileBackedCoordinatorStore {
    fn register_node(&self, node: StorageNodeInfo) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.nodes.insert(node.node_id.clone(), node);
        }
        self.persist()
    }

    fn get_node(&self, node_id: &str) -> Result<Option<StorageNodeInfo>> {
        let snap = self.state.read().unwrap();
        Ok(snap.nodes.get(node_id).cloned())
    }

    fn list_nodes(&self) -> Result<Vec<StorageNodeInfo>> {
        let snap = self.state.read().unwrap();
        Ok(snap.nodes.values().cloned().collect())
    }

    fn update_node_status(&self, node_id: &str, status: NodeStatus) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(node) = snap.nodes.get_mut(node_id) {
                node.transition_to(status)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            }
        }
        self.persist()
    }

    fn record_heartbeat(&self, heartbeat: NodeHeartbeat) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(node) = snap.nodes.get_mut(&heartbeat.node_id) {
                node.last_heartbeat_nanos = heartbeat.timestamp_nanos;
                node.capacity = heartbeat.capacity;
                node.active_tasks = heartbeat.active_replication_tasks;
                node.error_count = heartbeat.error_counter;
                if node.status == NodeStatus::Suspect || node.status == NodeStatus::Joining {
                    node.status = NodeStatus::Healthy;
                }
            }
        }
        self.persist()
    }

    fn create_token(&self, token: EnrollmentToken) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.tokens.insert(token.token.clone(), token);
        }
        self.persist()
    }

    fn get_token(&self, token_str: &str) -> Result<Option<EnrollmentToken>> {
        let snap = self.state.read().unwrap();
        Ok(snap.tokens.get(token_str).cloned())
    }

    fn consume_token(&self, token_str: &str) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.used = true;
            }
        }
        self.persist()
    }

    fn revoke_token(&self, token_str: &str) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.revoked = true;
            }
        }
        self.persist()
    }

    fn create_repository(&self, repo: RepositoryMeta) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.repositories.insert(repo.repo_id.clone(), repo);
        }
        self.persist()
    }

    fn get_repository(&self, repo_id: &str) -> Result<Option<RepositoryMeta>> {
        let snap = self.state.read().unwrap();
        Ok(snap.repositories.get(repo_id).cloned())
    }

    fn list_repositories(&self) -> Result<Vec<RepositoryMeta>> {
        let snap = self.state.read().unwrap();
        Ok(snap.repositories.values().cloned().collect())
    }

    fn set_placement(&self, placement: PlacementPlan) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            let repo_id = placement.repo_id.clone();
            snap.placements.insert(repo_id.clone(), placement);
        }
        self.persist()
    }

    fn get_placement(&self, repo_id: &str) -> Result<Option<PlacementPlan>> {
        let snap = self.state.read().unwrap();
        Ok(snap.placements.get(repo_id).cloned())
    }

    fn get_replicas(&self, repo_id: &str) -> Result<Vec<ReplicaState>> {
        let snap = self.state.read().unwrap();
        Ok(snap.replicas.get(repo_id).cloned().unwrap_or_default())
    }

    fn update_replica_state(
        &self,
        repo_id: &str,
        node_id: &str,
        is_primary: bool,
        last_synced_generation: u64,
        verified: bool,
        now_nanos: i64,
    ) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            let reps = snap.replicas.entry(repo_id.to_string()).or_default();
            if let Some(existing) = reps.iter_mut().find(|r| r.node_id == node_id) {
                existing.is_primary = is_primary;
                existing.last_synced_generation = last_synced_generation;
                existing.verified = verified;
                existing.last_verified_at_nanos = now_nanos;
            } else {
                reps.push(ReplicaState {
                    node_id: node_id.to_string(),
                    is_primary,
                    last_synced_generation,
                    verified,
                    last_verified_at_nanos: now_nanos,
                });
            }
        }
        self.persist()
    }

    fn get_ownership_epoch(&self, repo_id: &str) -> Result<Option<OwnershipEpoch>> {
        let snap = self.state.read().unwrap();
        Ok(snap
            .repositories
            .get(repo_id)
            .map(|r| OwnershipEpoch(r.current_ownership_epoch)))
    }

    fn update_ownership_epoch(
        &self,
        repo_id: &str,
        epoch: u64,
        primary_node_id: &str,
    ) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(repo) = snap.repositories.get_mut(repo_id) {
                repo.current_ownership_epoch = epoch;
            }
            if let Some(placement) = snap.placements.get_mut(repo_id) {
                placement.primary_node_id = primary_node_id.to_string();
            }
        }
        self.persist()
    }

    fn advance_committed_generation(&self, repo_id: &str, new_gen: u64) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            if let Some(repo) = snap.repositories.get_mut(repo_id) {
                repo.committed_generation = new_gen;
            }
        }
        self.persist()
    }

    fn create_transaction(&self, tx: ReplicationTransaction) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.transactions.insert(tx.transaction_id.clone(), tx);
        }
        self.persist()
    }

    fn get_transaction(&self, tx_id: &str) -> Result<Option<ReplicationTransaction>> {
        let snap = self.state.read().unwrap();
        Ok(snap.transactions.get(tx_id).cloned())
    }

    fn update_transaction(&self, tx: ReplicationTransaction) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.transactions.insert(tx.transaction_id.clone(), tx);
        }
        self.persist()
    }

    fn list_active_transactions(&self) -> Result<Vec<ReplicationTransaction>> {
        let snap = self.state.read().unwrap();
        Ok(snap
            .transactions
            .values()
            .filter(|t| {
                matches!(
                    t.phase,
                    crate::transaction::TransactionPhase::Initiated
                        | crate::transaction::TransactionPhase::Preparing
                        | crate::transaction::TransactionPhase::Prepared
                        | crate::transaction::TransactionPhase::CommitDecided
                )
            })
            .cloned()
            .collect())
    }

    fn create_repair_job(&self, job: RepairJob) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.repair_jobs.insert(job.job_id.clone(), job);
        }
        self.persist()
    }

    fn list_repair_jobs(&self) -> Result<Vec<RepairJob>> {
        let snap = self.state.read().unwrap();
        Ok(snap.repair_jobs.values().cloned().collect())
    }

    fn create_tombstone(&self, tombstone: TombstoneRecord) -> Result<()> {
        {
            let mut snap = self.state.write().unwrap();
            snap.tombstones
                .insert(tombstone.tombstone_id.clone(), tombstone);
        }
        self.persist()
    }

    fn list_tombstones(&self) -> Result<Vec<TombstoneRecord>> {
        let snap = self.state.read().unwrap();
        Ok(snap.tombstones.values().cloned().collect())
    }
}
