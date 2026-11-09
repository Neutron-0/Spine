use crate::durability::DurabilityPolicy;
use crate::enrollment::EnrollmentToken;
use crate::failover::ReplicaState;
use crate::fencing::OwnershipEpoch;
use crate::gc::TombstoneRecord;
use crate::heartbeat::NodeHeartbeat;
use crate::node::{NodeStatus, StorageNodeInfo};
use crate::placement::PlacementPlan;
use crate::protocol::VolumeSnapshotManifest;
use crate::repair::RepairJob;
use crate::transaction::ReplicationTransaction;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

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

#[derive(Clone, Default, Serialize, Deserialize)]
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

    fn enter_maintenance(&self, fencing_generation: u64) -> Result<()>;
    fn exit_maintenance(&self, fencing_generation: u64) -> Result<()>;
    fn get_maintenance_status(&self) -> (bool, u64);
    fn capture_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>)>;
    fn stage_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        payload: &[u8],
    ) -> Result<()>;
    fn commit_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()>;
    fn rollback_restore(&self, snapshot_id: &str) -> Result<()>;
    fn finalize_restore(&self, snapshot_id: &str) -> Result<()>;
}

pub struct FileBackedCoordinatorStore {
    path: Option<PathBuf>,
    state: RwLock<StateSnapshot>,
    in_maintenance: Arc<AtomicBool>,
    fencing_generation: Arc<AtomicU64>,
    staged_restore: RwLock<Option<(String, u64, Vec<u8>)>>,
    previous_committed_state: RwLock<Option<(String, StateSnapshot, u64)>>,
}

impl FileBackedCoordinatorStore {
    pub fn new_in_memory() -> Self {
        Self {
            path: None,
            state: RwLock::new(StateSnapshot::default()),
            in_maintenance: Arc::new(AtomicBool::new(false)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            staged_restore: RwLock::new(None),
            previous_committed_state: RwLock::new(None),
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

        let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
        let fencing_path = parent_dir.join(".coordinator_fencing_generation");
        let initial_gen = if fencing_path.exists() {
            let s = fs::read_to_string(&fencing_path).with_context(|| {
                format!(
                    "Failed to read coordinator fencing generation from {}",
                    fencing_path.display()
                )
            })?;
            let gen = s.trim().parse::<u64>().map_err(|e| {
                anyhow::anyhow!(
                    "Corrupt coordinator fencing generation record in {}: {}",
                    fencing_path.display(),
                    e
                )
            })?;
            if gen == 0 {
                bail!(
                    "Invalid coordinator fencing generation 0 in {}",
                    fencing_path.display()
                );
            }
            gen
        } else {
            let _ = fs::write(&fencing_path, "1\n");
            1
        };

        Ok(Self {
            path: Some(p),
            state: RwLock::new(state),
            in_maintenance: Arc::new(AtomicBool::new(false)),
            fencing_generation: Arc::new(AtomicU64::new(initial_gen)),
            staged_restore: RwLock::new(None),
            previous_committed_state: RwLock::new(None),
        })
    }

    pub fn persist_fencing_generation(&self, gen: u64) -> Result<()> {
        let cur = self.fencing_generation.load(Ordering::SeqCst);
        let target = std::cmp::max(cur, gen);
        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let fencing_path = parent_dir.join(".coordinator_fencing_generation");
            let tmp = fencing_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            fs::write(&tmp, format!("{}\n", target))?;
            if fs::rename(&tmp, &fencing_path).is_err() {
                let _ = fs::remove_file(&fencing_path);
                fs::rename(&tmp, &fencing_path)?;
            }
        }
        self.fencing_generation.store(target, Ordering::SeqCst);
        Ok(())
    }

    fn check_maintenance(&self) -> Result<()> {
        if self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Coordinator mutations rejected: cluster is currently in maintenance mode");
        }
        Ok(())
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        // Record heartbeat only updates transient health metadata; allowed or rejected during maintenance
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.used = true;
            }
        }
        self.persist()
    }

    fn revoke_token(&self, token_str: &str) -> Result<()> {
        self.check_maintenance()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.revoked = true;
            }
        }
        self.persist()
    }

    fn create_repository(&self, repo: RepositoryMeta) -> Result<()> {
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(repo) = snap.repositories.get_mut(repo_id) {
                repo.committed_generation = new_gen;
            }
        }
        self.persist()
    }

    fn create_transaction(&self, tx: ReplicationTransaction) -> Result<()> {
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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
        self.check_maintenance()?;
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

    fn enter_maintenance(&self, fencing_generation: u64) -> Result<()> {
        self.in_maintenance.store(true, Ordering::SeqCst);
        self.persist_fencing_generation(fencing_generation)?;
        Ok(())
    }

    fn exit_maintenance(&self, fencing_generation: u64) -> Result<()> {
        self.persist_fencing_generation(fencing_generation)?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn get_maintenance_status(&self) -> (bool, u64) {
        (
            self.in_maintenance.load(Ordering::SeqCst),
            self.fencing_generation.load(Ordering::SeqCst),
        )
    }

    fn capture_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>)> {
        let snap = self.state.read().unwrap();
        let payload = serde_json::to_vec_pretty(&*snap)?;
        let sha256_checksum = format!("{:x}", sha2::Sha256::digest(&payload));
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let manifest = VolumeSnapshotManifest {
            participant_id: "coordinator".to_string(),
            volume_name: "coordinator_data".to_string(),
            snapshot_id: snapshot_id.to_string(),
            fencing_generation,
            file_count: 1,
            total_bytes: payload.len() as u64,
            sha256_checksum,
            created_at_nanos: now_nanos,
        };
        Ok((manifest, payload))
    }

    fn stage_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        payload: &[u8],
    ) -> Result<()> {
        // Validate JSON payload
        let _test_snap: StateSnapshot = serde_json::from_slice(payload)
            .with_context(|| "Invalid coordinator state snapshot JSON")?;
        let mut staged = self.staged_restore.write().unwrap();
        *staged = Some((
            snapshot_id.to_string(),
            fencing_generation,
            payload.to_vec(),
        ));
        Ok(())
    }

    fn commit_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()> {
        let payload = {
            let mut staged = self.staged_restore.write().unwrap();
            match staged.take() {
                Some((id, gen, bytes)) => {
                    if id != snapshot_id {
                        bail!(
                            "Staged restore snapshot ID mismatch: expected {}, staged {}",
                            snapshot_id,
                            id
                        );
                    }
                    if gen != fencing_generation {
                        bail!(
                            "Staged restore fencing generation mismatch: expected {}, staged {}",
                            fencing_generation,
                            gen
                        );
                    }
                    bytes
                }
                None => bail!("No staged restore found for coordinator"),
            }
        };

        let new_snap: StateSnapshot = serde_json::from_slice(&payload)?;
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        {
            let mut current = self.state.write().unwrap();
            let mut prev = self.previous_committed_state.write().unwrap();
            *prev = Some((snapshot_id.to_string(), current.clone(), cur_gen));
            *current = new_snap;
        }
        self.persist()?;
        self.persist_fencing_generation(fencing_generation)?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn rollback_restore(&self, snapshot_id: &str) -> Result<()> {
        let mut prev = self.previous_committed_state.write().unwrap();
        if let Some((id, old_snap, old_gen)) = prev.take() {
            if id == snapshot_id {
                {
                    let mut current = self.state.write().unwrap();
                    *current = old_snap;
                }
                self.persist()?;
                self.persist_fencing_generation(old_gen)?;
            }
        }
        let mut staged = self.staged_restore.write().unwrap();
        if let Some((id, _, _)) = staged.as_ref() {
            if id == snapshot_id {
                *staged = None;
            }
        }
        Ok(())
    }

    fn finalize_restore(&self, snapshot_id: &str) -> Result<()> {
        let mut prev = self.previous_committed_state.write().unwrap();
        if let Some((id, _, _)) = prev.as_ref() {
            if id == snapshot_id {
                *prev = None;
            }
        }
        let mut staged = self.staged_restore.write().unwrap();
        if let Some((id, _, _)) = staged.as_ref() {
            if id == snapshot_id {
                *staged = None;
            }
        }
        Ok(())
    }
}
