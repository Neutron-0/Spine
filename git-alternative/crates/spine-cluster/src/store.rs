use crate::durability::DurabilityPolicy;
use crate::enrollment::EnrollmentToken;
use crate::failover::ReplicaState;
use crate::fencing::OwnershipEpoch;
use crate::gc::TombstoneRecord;
use crate::heartbeat::NodeHeartbeat;
use crate::node::{NodeStatus, StorageNodeInfo};
use crate::placement::PlacementPlan;
use crate::protocol::{ParticipantAcknowledgement, VolumeSnapshotManifest};
use crate::repair::RepairJob;
use crate::transaction::ReplicationTransaction;
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};

pub struct CoordinatorWriterGuard {
    active_writers: Arc<AtomicUsize>,
}

impl Drop for CoordinatorWriterGuard {
    fn drop(&mut self) {
        self.active_writers.fetch_sub(1, Ordering::SeqCst);
    }
}

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
    fn get_maintenance_status(&self) -> (bool, u64, usize);
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
    ) -> Result<ParticipantAcknowledgement> {
        self.stage_restore_with_manifest(snapshot_id, fencing_generation, None, payload)
    }

    fn stage_restore_with_manifest(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        manifest: Option<&VolumeSnapshotManifest>,
        payload: &[u8],
    ) -> Result<ParticipantAcknowledgement>;
    fn commit_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()>;
    fn rollback_restore(&self, snapshot_id: &str) -> Result<()>;
    fn finalize_restore(&self, snapshot_id: &str) -> Result<()>;
    fn query_staged_restore(&self, snapshot_id: &str)
        -> Result<Option<ParticipantAcknowledgement>>;
}

pub struct FileBackedCoordinatorStore {
    path: Option<PathBuf>,
    state: RwLock<StateSnapshot>,
    in_maintenance: Arc<AtomicBool>,
    fencing_generation: Arc<AtomicU64>,
    active_writers: Arc<AtomicUsize>,
    mutation_barrier: Arc<RwLock<()>>,
    staged_restore: RwLock<Option<(String, u64, Vec<u8>)>>,
    previous_committed_state: RwLock<Option<(String, StateSnapshot, u64)>>,
}

#[cfg(windows)]
fn is_write_through_unsupported(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(650) | Some(50) | Some(1))
}

#[cfg(windows)]
fn win32_sync_file_handle(path: &Path) {
    if let Ok(file) = fs::OpenOptions::new().write(true).open(path) {
        let _ = file.sync_all();
    }
}

#[cfg(windows)]
fn platform_atomic_replace_inner(
    from: &Path,
    to: &Path,
    write_through: bool,
) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    let wide_from: Vec<u16> = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let wide_to: Vec<u16> = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();

    extern "system" {
        fn MoveFileExW(
            lpExistingFileName: *const u16,
            lpNewFileName: *const u16,
            dwFlags: u32,
        ) -> i32;
        fn ReplaceFileW(
            lpReplacedFileName: *const u16,
            lpReplacementFileName: *const u16,
            lpBackupFileName: *const u16,
            dwReplaceFlags: u32,
            lpExclude: *mut std::ffi::c_void,
            lpReserved: *mut std::ffi::c_void,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    const REPLACEFILE_WRITE_THROUGH: u32 = 0x1;

    let move_flags = if write_through {
        MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
    } else {
        MOVEFILE_REPLACE_EXISTING
    };
    let replace_flags = if write_through {
        REPLACEFILE_WRITE_THROUGH
    } else {
        0
    };

    if to.exists() {
        let ret = unsafe {
            ReplaceFileW(
                wide_to.as_ptr(),
                wide_from.as_ptr(),
                std::ptr::null(),
                replace_flags,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if ret != 0 {
            return Ok(());
        }
        let ret = unsafe { MoveFileExW(wide_from.as_ptr(), wide_to.as_ptr(), move_flags) };
        if ret != 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    } else {
        let ret = unsafe { MoveFileExW(wide_from.as_ptr(), wide_to.as_ptr(), move_flags) };
        if ret != 0 {
            return Ok(());
        }
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    win32_sync_file_handle(from);
    match platform_atomic_replace_inner(from, to, true) {
        Ok(()) => Ok(()),
        Err(err) if is_write_through_unsupported(&err) => {
            platform_atomic_replace_inner(from, to, false)?;
            win32_sync_file_handle(to);
            Ok(())
        }
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn platform_atomic_replace(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::rename(from, to)
}

pub fn safe_atomic_replace(from: &Path, to: &Path) -> Result<()> {
    #[cfg(unix)]
    let sync_dir = |p: &Path| -> Result<()> {
        if let Ok(p_file) = fs::File::open(p) {
            p_file
                .sync_all()
                .with_context(|| format!("Failed to sync directory {}", p.display()))?;
        }
        Ok(())
    };
    #[cfg(windows)]
    let sync_dir = |p: &Path| -> Result<()> {
        if !p.exists() {
            bail!("Directory does not exist for sync: {}", p.display());
        }
        Ok(())
    };

    platform_atomic_replace(from, to).with_context(|| {
        format!(
            "Failed atomic replace from {} to {}",
            from.display(),
            to.display()
        )
    })?;

    if !to.exists() || fs::metadata(to).is_err() {
        bail!(
            "Readback verification failed after replacing {}",
            to.display()
        );
    }

    if let Some(parent) = to.parent() {
        sync_dir(parent)?;
    }

    Ok(())
}

impl FileBackedCoordinatorStore {
    pub fn new_in_memory() -> Self {
        Self {
            path: None,
            state: RwLock::new(StateSnapshot::default()),
            in_maintenance: Arc::new(AtomicBool::new(false)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            active_writers: Arc::new(AtomicUsize::new(0)),
            mutation_barrier: Arc::new(RwLock::new(())),
            staged_restore: RwLock::new(None),
            previous_committed_state: RwLock::new(None),
        }
    }

    pub fn new_file_backed<P: AsRef<Path>>(path: P) -> Result<Self> {
        let p = path.as_ref().to_path_buf();
        let existed_before = p.exists();
        let state = if existed_before {
            let bytes = fs::read(&p).with_context(|| format!("Reading {}", p.display()))?;
            serde_json::from_slice(&bytes).unwrap_or_default()
        } else {
            if let Some(parent) = p.parent() {
                fs::create_dir_all(parent)?;
            }
            let initial = StateSnapshot::default();
            let bytes = serde_json::to_vec_pretty(&initial)?;
            let _ = fs::write(&p, bytes);
            initial
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
        } else if existed_before {
            bail!(
                "FATAL: Coordinator state file '{}' exists, but fencing generation record '{}' is missing! Existing participants with missing fencing markers must fail closed.",
                p.display(),
                fencing_path.display()
            );
        } else {
            let _ = fs::write(&fencing_path, "1\n");
            1
        };

        let mut initial_staged = None;
        let mut initial_prev = None;
        if let Ok(entries) = fs::read_dir(parent_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with(".staged_coordinator_restore_") && name.ends_with(".json") {
                    if let Ok(bytes) = fs::read(entry.path()) {
                        if let Ok(record) = serde_json::from_slice::<(String, u64, Vec<u8>)>(&bytes)
                        {
                            initial_staged = Some(record);
                        }
                    }
                } else if name.starts_with(".previous_coordinator_state_")
                    && name.ends_with(".json")
                {
                    if let Ok(bytes) = fs::read(entry.path()) {
                        if let Ok(record) =
                            serde_json::from_slice::<(String, StateSnapshot, u64)>(&bytes)
                        {
                            initial_prev = Some(record);
                        }
                    }
                }
            }
        }

        Ok(Self {
            path: Some(p),
            state: RwLock::new(state),
            in_maintenance: Arc::new(AtomicBool::new(false)),
            fencing_generation: Arc::new(AtomicU64::new(initial_gen)),
            active_writers: Arc::new(AtomicUsize::new(0)),
            mutation_barrier: Arc::new(RwLock::new(())),
            staged_restore: RwLock::new(initial_staged),
            previous_committed_state: RwLock::new(initial_prev),
        })
    }

    pub fn persist_fencing_generation(&self, gen: u64) -> Result<()> {
        let cur = self.fencing_generation.load(Ordering::SeqCst);
        if gen < cur {
            bail!(
                "Monotonicity violation: cannot decrement fencing generation from {} to {}",
                cur,
                gen
            );
        }
        let target = std::cmp::max(cur, gen);
        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let fencing_path = parent_dir.join(".coordinator_fencing_generation");
            let tmp = fencing_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            {
                let mut f = fs::File::create(&tmp)?;
                use std::io::Write;
                f.write_all(format!("{}\n", target).as_bytes())?;
                f.flush()?;
                f.sync_all()?;
            }
            if let Err(e) = safe_atomic_replace(&tmp, &fencing_path) {
                let _ = fs::remove_file(&tmp);
                bail!(
                    "Failed to durably persist coordinator fencing generation {}: {}",
                    target,
                    e
                );
            }
        }
        self.fencing_generation.store(target, Ordering::SeqCst);
        Ok(())
    }

    pub fn acquire_writer(&self) -> Result<CoordinatorWriterGuard> {
        let _barrier = self
            .mutation_barrier
            .read()
            .map_err(|_| anyhow::anyhow!("Coordinator mutation barrier poisoned"))?;
        if self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Coordinator mutations rejected: cluster is currently in maintenance mode");
        }
        self.active_writers.fetch_add(1, Ordering::SeqCst);
        Ok(CoordinatorWriterGuard {
            active_writers: Arc::clone(&self.active_writers),
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.used = true;
            }
        }
        self.persist()
    }

    fn revoke_token(&self, token_str: &str) -> Result<()> {
        let _guard = self.acquire_writer()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(tok) = snap.tokens.get_mut(token_str) {
                tok.revoked = true;
            }
        }
        self.persist()
    }

    fn create_repository(&self, repo: RepositoryMeta) -> Result<()> {
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
        {
            let mut snap = self.state.write().unwrap();
            if let Some(repo) = snap.repositories.get_mut(repo_id) {
                repo.committed_generation = new_gen;
            }
        }
        self.persist()
    }

    fn create_transaction(&self, tx: ReplicationTransaction) -> Result<()> {
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _guard = self.acquire_writer()?;
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
        let _barrier = self
            .mutation_barrier
            .write()
            .map_err(|_| anyhow::anyhow!("Coordinator mutation barrier poisoned"))?;
        self.in_maintenance.store(true, Ordering::SeqCst);
        self.persist_fencing_generation(fencing_generation)?;

        let start = std::time::Instant::now();
        let timeout = std::time::Duration::from_secs(5);
        while self.active_writers.load(Ordering::SeqCst) > 0 {
            if start.elapsed() > timeout {
                bail!(
                    "Timed out waiting for {} active coordinator writers to drain during enter_maintenance",
                    self.active_writers.load(Ordering::SeqCst)
                );
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Ok(())
    }

    fn exit_maintenance(&self, fencing_generation: u64) -> Result<()> {
        let _barrier = self
            .mutation_barrier
            .write()
            .map_err(|_| anyhow::anyhow!("Coordinator mutation barrier poisoned"))?;
        self.persist_fencing_generation(fencing_generation)?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(())
    }

    fn get_maintenance_status(&self) -> (bool, u64, usize) {
        (
            self.in_maintenance.load(Ordering::SeqCst),
            self.fencing_generation.load(Ordering::SeqCst),
            self.active_writers.load(Ordering::SeqCst),
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
            restore_id: None,
            backup_id: Some(snapshot_id.to_string()),
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
    ) -> Result<ParticipantAcknowledgement> {
        self.stage_restore_with_manifest(snapshot_id, fencing_generation, None, payload)
    }

    fn stage_restore_with_manifest(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
        manifest: Option<&VolumeSnapshotManifest>,
        payload: &[u8],
    ) -> Result<ParticipantAcknowledgement> {
        if !self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Coordinator must be in maintenance mode to stage restore");
        }
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        if fencing_generation < cur_gen {
            bail!(
                "Cannot stage restore with stale fencing generation {} < {}",
                fencing_generation,
                cur_gen
            );
        }

        // Validate JSON payload
        let _test_snap: StateSnapshot = serde_json::from_slice(payload)
            .with_context(|| "Invalid coordinator state snapshot JSON")?;
        let staged_record = (
            snapshot_id.to_string(),
            fencing_generation,
            payload.to_vec(),
        );

        let restore_id = manifest
            .and_then(|m| m.restore_id.clone())
            .unwrap_or_else(|| format!("rst_{}", snapshot_id));
        let backup_id = manifest
            .and_then(|m| m.backup_id.clone())
            .unwrap_or_else(|| snapshot_id.to_string());

        use sha2::Sha256;
        let mut hasher = Sha256::new();
        hasher.update(payload);
        let staged_digest = hex::encode(hasher.finalize());
        let now_nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0);

        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let staged_path =
                parent_dir.join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
            let tmp = staged_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            let json = serde_json::to_vec_pretty(&staged_record)?;
            {
                let mut f = fs::File::create(&tmp)?;
                use std::io::Write;
                f.write_all(&json)?;
                f.flush()?;
                f.sync_all()?;
            }
            if let Err(e) = safe_atomic_replace(&tmp, &staged_path) {
                let _ = fs::remove_file(&tmp);
                bail!("Failed to persist staged coordinator restore: {}", e);
            }

            let meta_path = parent_dir.join(format!(
                ".staged_coordinator_restore_record_{}.json",
                snapshot_id
            ));
            let meta_tmp = meta_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            let meta_json = serde_json::json!({
                "restore_id": restore_id,
                "backup_id": backup_id,
                "snapshot_id": snapshot_id,
                "fencing_generation": fencing_generation,
                "staged_digest": staged_digest,
                "staged_bytes": payload.len(),
                "staged_at_nanos": now_nanos,
            });
            if let Ok(bytes) = serde_json::to_vec_pretty(&meta_json) {
                if let Ok(mut f) = fs::File::create(&meta_tmp) {
                    use std::io::Write;
                    let _ = f.write_all(&bytes);
                    let _ = f.flush();
                    let _ = f.sync_all();
                    let _ = safe_atomic_replace(&meta_tmp, &meta_path);
                }
            }
        }

        let mut staged = self.staged_restore.write().unwrap();
        *staged = Some(staged_record);

        let ack = ParticipantAcknowledgement {
            restore_id,
            backup_id,
            participant_id: "coordinator".to_string(),
            volume_name: "coordinator_data".to_string(),
            canonical_resource_id: "coordinator_cluster_state".to_string(),
            previous_generation: Some(cur_gen.to_string()),
            target_generation: fencing_generation.to_string(),
            staged_digest: staged_digest.clone(),
            archive_payload_sha256: staged_digest.clone(),
            staged_resource_digest: staged_digest,
            staged_byte_length: payload.len() as u64,
            prepared_at_nanos: now_nanos,
            prepared: true,
            durable_staging_record_id: format!(".staged_coordinator_restore_{}.json", snapshot_id),
            current_state: "Staged".to_string(),
        };

        Ok(ack)
    }

    fn commit_restore(&self, snapshot_id: &str, fencing_generation: u64) -> Result<()> {
        if !self.in_maintenance.load(Ordering::SeqCst) {
            bail!("Coordinator must be in maintenance mode to commit restore");
        }
        let cur_gen = self.fencing_generation.load(Ordering::SeqCst);
        if fencing_generation < cur_gen {
            bail!(
                "Cannot commit restore with stale fencing generation {} < {}",
                fencing_generation,
                cur_gen
            );
        }

        let payload = {
            let mut staged = self.staged_restore.write().unwrap();
            match staged.take() {
                Some((id, gen, bytes)) => {
                    if id != snapshot_id {
                        *staged = Some((id.clone(), gen, bytes));
                        bail!(
                            "Staged restore snapshot ID mismatch: expected {}, staged {}",
                            snapshot_id,
                            id
                        );
                    }
                    if gen != fencing_generation {
                        *staged = Some((id, gen, bytes));
                        bail!(
                            "Staged restore fencing generation mismatch: expected {}, staged {}",
                            fencing_generation,
                            gen
                        );
                    }
                    bytes
                }
                None => {
                    if let Some(ref p) = self.path {
                        let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
                        let staged_path = parent_dir
                            .join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
                        if staged_path.exists() {
                            let bytes = fs::read(&staged_path)?;
                            let (id, gen, payload): (String, u64, Vec<u8>) =
                                serde_json::from_slice(&bytes)?;
                            if id != snapshot_id {
                                bail!(
                                    "Disk staged restore snapshot ID mismatch: expected {}, staged {}",
                                    snapshot_id,
                                    id
                                );
                            }
                            if gen != fencing_generation {
                                bail!(
                                    "Disk staged restore fencing generation mismatch: expected {}, staged {}",
                                    fencing_generation,
                                    gen
                                );
                            }
                            payload
                        } else {
                            bail!("No staged restore found for coordinator");
                        }
                    } else {
                        bail!("No staged restore found for coordinator");
                    }
                }
            }
        };

        let new_snap: StateSnapshot = serde_json::from_slice(&payload)?;
        let prev_record = (
            snapshot_id.to_string(),
            self.state.read().unwrap().clone(),
            cur_gen,
        );

        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let prev_path =
                parent_dir.join(format!(".previous_coordinator_state_{}.json", snapshot_id));
            let tmp = prev_path.with_extension(format!("tmp.{}", uuid::Uuid::new_v4()));
            let json = serde_json::to_vec_pretty(&prev_record)?;
            {
                let mut f = fs::File::create(&tmp)?;
                use std::io::Write;
                f.write_all(&json)?;
                f.flush()?;
                f.sync_all()?;
            }
            if let Err(e) = safe_atomic_replace(&tmp, &prev_path) {
                let _ = fs::remove_file(&tmp);
                bail!("Failed to persist previous coordinator state backup: {}", e);
            }

            let staged_path =
                parent_dir.join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
            let _ = fs::remove_file(&staged_path);
        }

        {
            let mut current = self.state.write().unwrap();
            let mut prev = self.previous_committed_state.write().unwrap();
            *prev = Some(prev_record);
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

        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let prev_path =
                parent_dir.join(format!(".previous_coordinator_state_{}.json", snapshot_id));
            let _ = fs::remove_file(&prev_path);
            let staged_path =
                parent_dir.join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
            let _ = fs::remove_file(&staged_path);
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

        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let prev_path =
                parent_dir.join(format!(".previous_coordinator_state_{}.json", snapshot_id));
            let _ = fs::remove_file(&prev_path);
            let staged_path =
                parent_dir.join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
            let _ = fs::remove_file(&staged_path);
            let meta_path = parent_dir.join(format!(
                ".staged_coordinator_restore_record_{}.json",
                snapshot_id
            ));
            let _ = fs::remove_file(&meta_path);
        }
        Ok(())
    }

    fn query_staged_restore(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<ParticipantAcknowledgement>> {
        let mut restore_id = format!("rst_{}", snapshot_id);
        let mut backup_id = snapshot_id.to_string();

        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let meta_path = parent_dir.join(format!(
                ".staged_coordinator_restore_record_{}.json",
                snapshot_id
            ));
            if meta_path.exists() {
                if let Ok(bytes) = fs::read(&meta_path) {
                    if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(r) = val.get("restore_id").and_then(|v| v.as_str()) {
                            restore_id = r.to_string();
                        }
                        if let Some(b) = val.get("backup_id").and_then(|v| v.as_str()) {
                            backup_id = b.to_string();
                        }
                    }
                }
            }
        }

        // 1. Check in-memory staged_restore
        {
            let staged = self.staged_restore.read().unwrap();
            if let Some((ref s_id, gen, ref payload)) = *staged {
                if s_id == snapshot_id {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(payload);
                    let staged_digest = hex::encode(hasher.finalize());
                    let ack = ParticipantAcknowledgement {
                        restore_id: restore_id.clone(),
                        backup_id: backup_id.clone(),
                        participant_id: "coordinator".to_string(),
                        volume_name: "coordinator_data".to_string(),
                        canonical_resource_id: "coordinator_cluster_state".to_string(),
                        previous_generation: None,
                        target_generation: gen.to_string(),
                        staged_digest: staged_digest.clone(),
                        archive_payload_sha256: staged_digest.clone(),
                        staged_resource_digest: staged_digest,
                        staged_byte_length: payload.len() as u64,
                        prepared_at_nanos: 0,
                        prepared: true,
                        durable_staging_record_id: format!(
                            ".staged_coordinator_restore_{}.json",
                            snapshot_id
                        ),
                        current_state: "Staged".to_string(),
                    };
                    return Ok(Some(ack));
                }
            }
        }

        // 2. Check disk staged restore record
        if let Some(ref p) = self.path {
            let parent_dir = p.parent().unwrap_or_else(|| Path::new("."));
            let staged_path =
                parent_dir.join(format!(".staged_coordinator_restore_{}.json", snapshot_id));
            if staged_path.exists() {
                let bytes = fs::read(&staged_path)?;
                let (s_id, gen, payload): (String, u64, Vec<u8>) = serde_json::from_slice(&bytes)?;
                if s_id == snapshot_id {
                    use sha2::{Digest, Sha256};
                    let mut hasher = Sha256::new();
                    hasher.update(&payload);
                    let staged_digest = hex::encode(hasher.finalize());
                    let ack = ParticipantAcknowledgement {
                        restore_id,
                        backup_id,
                        participant_id: "coordinator".to_string(),
                        volume_name: "coordinator_data".to_string(),
                        canonical_resource_id: "coordinator_cluster_state".to_string(),
                        previous_generation: None,
                        target_generation: gen.to_string(),
                        staged_digest: staged_digest.clone(),
                        archive_payload_sha256: staged_digest.clone(),
                        staged_resource_digest: staged_digest,
                        staged_byte_length: payload.len() as u64,
                        prepared_at_nanos: 0,
                        prepared: true,
                        durable_staging_record_id: format!(
                            ".staged_coordinator_restore_{}.json",
                            snapshot_id
                        ),
                        current_state: "Staged".to_string(),
                    };
                    return Ok(Some(ack));
                }
            }
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coordinator_writer_guard_and_maintenance_barrier() {
        let store = FileBackedCoordinatorStore::new_in_memory();
        assert_eq!(store.get_maintenance_status(), (false, 1, 0));

        {
            let _guard = store
                .acquire_writer()
                .expect("Acquire writer should succeed");
            assert_eq!(store.get_maintenance_status(), (false, 1, 1));
        }
        assert_eq!(store.get_maintenance_status(), (false, 1, 0));

        // Enter maintenance barrier
        store
            .enter_maintenance(5)
            .expect("Enter maintenance should succeed");
        assert_eq!(store.get_maintenance_status(), (true, 5, 0));

        // Attempt mutation while in maintenance must fail
        let mut_res = store.acquire_writer();
        assert!(
            mut_res.is_err(),
            "Writer must be rejected during maintenance"
        );

        // Exit maintenance
        store
            .exit_maintenance(5)
            .expect("Exit maintenance should succeed");
        assert_eq!(store.get_maintenance_status(), (false, 5, 0));

        // Now writer succeeds again
        let _guard = store
            .acquire_writer()
            .expect("Writer should succeed after exit maintenance");
        assert_eq!(store.get_maintenance_status(), (false, 5, 1));
    }
}
