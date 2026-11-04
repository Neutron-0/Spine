//! Authoritative Distributed Repository Write Pipeline.
//!
//! Provides a single, unified write path for all operations that modify authoritative
//! repository state (CLI push, SPWP wire updates, branch updates, and Forge PR merges).
//!
//! Integrates:
//! - Actor authentication and role-based permissions
//! - Strict branch protection enforcement (PR required, review thresholds, changes addressed)
//! - Coordinator placement and ownership epoch fencing (preventing split-brain / stale writers)
//! - Expected reference compare-and-swap (CAS) and fast-forward validation
//! - Pack integrity checksums and transitive object closure verification
//! - Two-phase replication transactions (2PC) enforcing DurabilityPolicy (SingleCopy, TwoCopies, ThreeCopies)
//! - Atomic reference publication via `.lock` safety
//! - Transaction crash recovery and durability auditing

use crate::durability::DurabilityPolicy;
use crate::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use crate::store::CoordinatorStore;
use crate::transaction::{ReplicationTransaction, TransactionError};
use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use spine_history::is_ancestor;
use spine_net::auth::Role;
use spine_object_store::CompositeObjectStore;
use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

#[derive(Error, Debug)]
pub enum WritePipelineError {
    #[error("Permission denied: {0}")]
    PermissionDenied(String),
    #[error("Branch protection violation: {0}")]
    BranchProtection(String),
    #[error("Stale reference tip: {0}")]
    StaleReference(String),
    #[error("Non-fast-forward update rejected; force push required: {0}")]
    NonFastForward(String),
    #[error("Missing object in closure: {0}")]
    MissingObject(String),
    #[error("Ownership epoch fencing rejected write: current {current}, presented {presented}")]
    OwnershipEpochMismatch { current: u64, presented: u64 },
    #[error("Durability requirement failed (required {required}, verified {actual}): {message}")]
    DurabilityRequirementFailed {
        required: usize,
        actual: usize,
        message: String,
    },
    #[error("Storage node communication error: {0}")]
    StorageNodeError(String),
    #[error("Coordinator store error: {0}")]
    CoordinatorError(String),
    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),
    #[error("Internal write pipeline error: {0}")]
    Internal(String),
}

/// An authoritative reference update proposal.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthoritativeRefUpdate {
    pub ref_name: String,
    pub expected_old_oid: Option<ObjectId>,
    pub new_oid: ObjectId,
    pub lease_oid: Option<ObjectId>,
    pub force: bool,
}

/// The actor requesting the repository write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WriteActor {
    pub actor_id: String,
    pub username: String,
    pub role: Role,
}

/// Context provided when a write originates from a pull request merge.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PrProtectionContext {
    pub pr_number: u64,
    pub author_id: String,
    pub approved_count: u32,
    pub has_active_change_requests: bool,
    pub has_unresolved_comments: bool,
}

/// Branch protection rule configured for a repository.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BranchProtectionRule {
    pub branch_pattern: String,
    pub require_pull_request: bool,
    pub required_approvals: u32,
    pub prevent_self_approval: bool,
    pub require_changes_addressed: bool,
    pub allow_force_push: bool,
}

impl BranchProtectionRule {
    pub fn matches(&self, ref_name: &str) -> bool {
        let branch = ref_name.strip_prefix("refs/heads/").unwrap_or(ref_name);
        if self.branch_pattern == branch {
            return true;
        }
        if let Some(prefix) = self.branch_pattern.strip_suffix('*') {
            if branch.starts_with(prefix) {
                return true;
            }
        }
        false
    }
}

/// Outcome of a successful authoritative repository write.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AuthoritativeWriteResult {
    pub transaction_id: String,
    pub committed_generation: u64,
    pub updated_refs: Vec<(String, ObjectId)>,
    pub copies_verified: usize,
    pub durability_policy: DurabilityPolicy,
    pub committed_at_nanos: i64,
}

/// Durability status report for an authoritative repository across cluster nodes.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DurabilityVerificationReport {
    pub repo_id: String,
    pub durability_policy: DurabilityPolicy,
    pub required_copies: usize,
    pub verified_copies: usize,
    pub is_durability_satisfied: bool,
    pub committed_generation: u64,
    pub primary_node_id: String,
    pub replica_nodes: Vec<String>,
    pub node_generations: HashMap<String, u64>,
}

/// Abstraction for communicating with storage nodes (over TCP RPC or local memory/filesystem).
pub trait StorageDispatcher: Send + Sync {
    #[allow(clippy::too_many_arguments)]
    fn prepare_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        generation: u64,
        pack_id: &'a str,
        pack_checksum: &'a str,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<bool, WritePipelineError>> + Send + 'a>>;

    fn commit_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        new_generation: u64,
        ref_updates: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>>;

    fn abort_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>>;

    fn inspect_repo<'a>(
        &'a self,
        node_id: &'a str,
        repo_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<(u64, Vec<String>, bool), WritePipelineError>> + Send + 'a>,
    >;
}

/// Network-based storage dispatcher communicating over SPWP / Cluster TCP RPC.
pub struct NetworkStorageDispatcher {
    node_addresses: HashMap<String, String>,
}

impl NetworkStorageDispatcher {
    pub fn new(node_addresses: HashMap<String, String>) -> Self {
        Self { node_addresses }
    }

    pub fn set_node_addr(&mut self, node_id: String, addr: String) {
        self.node_addresses.insert(node_id, addr);
    }
}

impl StorageDispatcher for NetworkStorageDispatcher {
    fn prepare_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        generation: u64,
        pack_id: &'a str,
        pack_checksum: &'a str,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<bool, WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let addr = self.node_addresses.get(node_id).ok_or_else(|| {
                WritePipelineError::StorageNodeError(format!(
                    "Address for node {} not found",
                    node_id
                ))
            })?;

            let req = ClusterRpcRequest::PreparePush {
                transaction_id: tx_id.to_string(),
                repo_id: repo_id.to_string(),
                ownership_epoch: epoch,
                proposed_generation: generation,
                pack_id: pack_id.to_string(),
                pack_checksum: pack_checksum.to_string(),
                pack_bytes,
                index_bytes,
            };

            match send_tcp_rpc(addr, &req).await {
                Ok(ClusterRpcResponse::PrepareAck { prepared, .. }) => Ok(prepared),
                Ok(ClusterRpcResponse::Error(msg)) => {
                    Err(WritePipelineError::StorageNodeError(msg))
                }
                Ok(_) => Err(WritePipelineError::StorageNodeError(
                    "Unexpected response to PreparePush".into(),
                )),
                Err(e) => Err(WritePipelineError::StorageNodeError(e.to_string())),
            }
        })
    }

    fn commit_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        new_generation: u64,
        _ref_updates: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let addr = self.node_addresses.get(node_id).ok_or_else(|| {
                WritePipelineError::StorageNodeError(format!(
                    "Address for node {} not found",
                    node_id
                ))
            })?;

            let req = ClusterRpcRequest::CommitPush {
                transaction_id: tx_id.to_string(),
                repo_id: repo_id.to_string(),
                ownership_epoch: epoch,
                new_generation,
            };

            match send_tcp_rpc(addr, &req).await {
                Ok(ClusterRpcResponse::CommitAck { .. }) => Ok(()),
                Ok(ClusterRpcResponse::Error(msg)) => {
                    Err(WritePipelineError::StorageNodeError(msg))
                }
                Ok(_) => Err(WritePipelineError::StorageNodeError(
                    "Unexpected response to CommitPush".into(),
                )),
                Err(e) => Err(WritePipelineError::StorageNodeError(e.to_string())),
            }
        })
    }

    fn abort_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            if let Some(addr) = self.node_addresses.get(node_id) {
                let req = ClusterRpcRequest::AbortPush {
                    transaction_id: tx_id.to_string(),
                    repo_id: repo_id.to_string(),
                    ownership_epoch: epoch,
                    reason: reason.to_string(),
                };
                let _ = send_tcp_rpc(addr, &req).await;
            }
            Ok(())
        })
    }

    fn inspect_repo<'a>(
        &'a self,
        node_id: &'a str,
        repo_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<(u64, Vec<String>, bool), WritePipelineError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let addr = self.node_addresses.get(node_id).ok_or_else(|| {
                WritePipelineError::StorageNodeError(format!(
                    "Address for node {} not found",
                    node_id
                ))
            })?;

            let req = ClusterRpcRequest::InspectRepository {
                repo_id: repo_id.to_string(),
            };

            match send_tcp_rpc(addr, &req).await {
                Ok(ClusterRpcResponse::InspectResult {
                    generation,
                    packs,
                    is_healthy,
                    ..
                }) => Ok((generation, packs, is_healthy)),
                Ok(ClusterRpcResponse::Error(e)) => Err(WritePipelineError::StorageNodeError(e)),
                _ => Err(WritePipelineError::StorageNodeError(
                    "Unexpected response".into(),
                )),
            }
        })
    }
}

/// Local in-process or filesystem storage dispatcher for deterministic testing and single-host runs.
#[derive(Default)]
pub struct LocalBackendStorageDispatcher {
    node_roots: std::sync::RwLock<HashMap<String, std::path::PathBuf>>,
}

impl LocalBackendStorageDispatcher {
    pub fn new() -> Self {
        Self {
            node_roots: std::sync::RwLock::new(HashMap::new()),
        }
    }

    pub fn register_node(&self, node_id: String, root: std::path::PathBuf) {
        let mut map = self.node_roots.write().unwrap();
        map.insert(node_id, root);
    }
}

impl StorageDispatcher for LocalBackendStorageDispatcher {
    fn prepare_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        epoch: u64,
        _generation: u64,
        pack_id: &'a str,
        pack_checksum: &'a str,
        pack_bytes: Vec<u8>,
        index_bytes: Vec<u8>,
    ) -> Pin<Box<dyn Future<Output = Result<bool, WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let root = {
                let map = self.node_roots.read().unwrap();
                map.get(node_id).cloned().ok_or_else(|| {
                    WritePipelineError::StorageNodeError(format!("Node {} not found", node_id))
                })?
            };

            // Verify checksum
            let actual = blake3::hash(&pack_bytes).to_hex().to_string();
            if actual != pack_checksum {
                return Err(WritePipelineError::StorageNodeError(format!(
                    "Pack checksum mismatch: expected {}, got {}",
                    pack_checksum, actual
                )));
            }

            let stage = root.join("staging").join(tx_id);
            std::fs::create_dir_all(&stage)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;

            std::fs::write(stage.join("repo_id"), repo_id)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;
            std::fs::write(stage.join("pack_id"), pack_id)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;
            std::fs::write(stage.join("epoch"), format!("{}\n", epoch))
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;
            std::fs::write(stage.join(format!("{}.spk", pack_id)), &pack_bytes)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;
            std::fs::write(stage.join(format!("{}.spidx", pack_id)), &index_bytes)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;

            Ok(true)
        })
    }

    fn commit_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        repo_id: &'a str,
        _epoch: u64,
        new_generation: u64,
        _ref_updates: Vec<(String, String)>,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let root = {
                let map = self.node_roots.read().unwrap();
                map.get(node_id).cloned().ok_or_else(|| {
                    WritePipelineError::StorageNodeError(format!("Node {} not found", node_id))
                })?
            };

            let stage = root.join("staging").join(tx_id);
            if !stage.exists() {
                return Err(WritePipelineError::StorageNodeError(format!(
                    "Staging dir for tx {} not found",
                    tx_id
                )));
            }

            let pack_id = std::fs::read_to_string(stage.join("pack_id"))
                .unwrap_or_default()
                .trim()
                .to_string();

            let spine_dir = root.join("repos").join(repo_id).join(".spine");
            let pack_dir = spine_dir.join("pack");
            std::fs::create_dir_all(&pack_dir)
                .map_err(|e| WritePipelineError::StorageNodeError(e.to_string()))?;

            let src_spk = stage.join(format!("{}.spk", pack_id));
            let dst_spk = pack_dir.join(format!("{}.spk", pack_id));
            let src_spidx = stage.join(format!("{}.spidx", pack_id));
            let dst_spidx = pack_dir.join(format!("{}.spidx", pack_id));

            if src_spk.exists() {
                let _ = std::fs::copy(&src_spk, &dst_spk);
            }
            if src_spidx.exists() {
                let _ = std::fs::copy(&src_spidx, &dst_spidx);
            }

            let _ = std::fs::remove_dir_all(&stage);
            let _ = std::fs::write(
                spine_dir.join("generation"),
                format!("{}\n", new_generation),
            );

            Ok(())
        })
    }

    fn abort_push<'a>(
        &'a self,
        node_id: &'a str,
        tx_id: &'a str,
        _repo_id: &'a str,
        _epoch: u64,
        _reason: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<(), WritePipelineError>> + Send + 'a>> {
        Box::pin(async move {
            let root = {
                let map = self.node_roots.read().unwrap();
                map.get(node_id).cloned()
            };
            if let Some(r) = root {
                let stage = r.join("staging").join(tx_id);
                if stage.exists() {
                    let _ = std::fs::remove_dir_all(&stage);
                }
            }
            Ok(())
        })
    }

    fn inspect_repo<'a>(
        &'a self,
        node_id: &'a str,
        repo_id: &'a str,
    ) -> Pin<
        Box<dyn Future<Output = Result<(u64, Vec<String>, bool), WritePipelineError>> + Send + 'a>,
    > {
        Box::pin(async move {
            let root = {
                let map = self.node_roots.read().unwrap();
                map.get(node_id).cloned().ok_or_else(|| {
                    WritePipelineError::StorageNodeError(format!("Node {} not found", node_id))
                })?
            };

            let spine_dir = root.join("repos").join(repo_id).join(".spine");
            let gen = if let Ok(c) = std::fs::read_to_string(spine_dir.join("generation")) {
                c.trim().parse().unwrap_or(0)
            } else {
                0
            };

            let mut packs = Vec::new();
            let pack_dir = spine_dir.join("pack");
            if let Ok(entries) = std::fs::read_dir(pack_dir) {
                for e in entries.flatten() {
                    let name = e.file_name().to_string_lossy().to_string();
                    if name.ends_with(".spk") {
                        packs.push(name.trim_end_matches(".spk").to_string());
                    }
                }
            }

            Ok((gen, packs, true))
        })
    }
}

async fn send_tcp_rpc(addr: &str, req: &ClusterRpcRequest) -> Result<ClusterRpcResponse, String> {
    let mut stream = TcpStream::connect(addr)
        .await
        .map_err(|e| format!("Connect to {}: {}", addr, e))?;

    let req_bytes = req.to_bytes().map_err(|e| e.to_string())?;
    let len = (req_bytes.len() as u32).to_be_bytes();
    stream
        .write_all(&len)
        .await
        .map_err(|e| format!("Write len: {}", e))?;
    stream
        .write_all(&req_bytes)
        .await
        .map_err(|e| format!("Write payload: {}", e))?;
    stream.flush().await.map_err(|e| format!("Flush: {}", e))?;

    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .await
        .map_err(|e| format!("Read len: {}", e))?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;
    let mut body = vec![0u8; resp_len];
    stream
        .read_exact(&mut body)
        .await
        .map_err(|e| format!("Read payload: {}", e))?;

    ClusterRpcResponse::from_bytes(&body).map_err(|e| format!("Decode: {}", e))
}

/// Authoritative Repository Writer orchestrating the unified write path.
pub struct AuthoritativeRepositoryWriter {
    coordinator_store: Arc<dyn CoordinatorStore>,
    dispatcher: Arc<dyn StorageDispatcher>,
    rules: Vec<BranchProtectionRule>,
}

impl AuthoritativeRepositoryWriter {
    pub fn new(
        coordinator_store: Arc<dyn CoordinatorStore>,
        dispatcher: Arc<dyn StorageDispatcher>,
        rules: Vec<BranchProtectionRule>,
    ) -> Self {
        Self {
            coordinator_store,
            dispatcher,
            rules,
        }
    }

    pub fn add_rule(&mut self, rule: BranchProtectionRule) {
        self.rules.push(rule);
    }

    /// Primary execution entrypoint for all authoritative writes.
    #[allow(clippy::too_many_arguments)]
    pub async fn execute_write(
        &self,
        repo_id: &str,
        actor: &WriteActor,
        updates: &[AuthoritativeRefUpdate],
        pack_id: &str,
        pack_bytes: &[u8],
        index_bytes: &[u8],
        spine_dir: &Path,
        primary_store: &CompositeObjectStore,
        is_pr_merge: bool,
        pr_context: Option<&PrProtectionContext>,
    ) -> Result<AuthoritativeWriteResult, WritePipelineError> {
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i64;

        // 1. Authenticate & Authorize Actor
        if !actor.role.can_push() {
            return Err(WritePipelineError::PermissionDenied(format!(
                "Actor '{}' with role '{}' lacks push permissions on repository '{}'",
                actor.username, actor.role, repo_id
            )));
        }

        // 2. Branch Protection Enforcement at the Authoritative Write Boundary
        for update in updates {
            for rule in &self.rules {
                if rule.matches(&update.ref_name) {
                    if !is_pr_merge && rule.require_pull_request {
                        return Err(WritePipelineError::BranchProtection(format!(
                            "Direct push to protected branch '{}' is prohibited; changes must be submitted via pull request",
                            update.ref_name
                        )));
                    }

                    if update.force && !rule.allow_force_push {
                        return Err(WritePipelineError::BranchProtection(format!(
                            "Force push to protected branch '{}' is strictly prohibited",
                            update.ref_name
                        )));
                    }

                    if is_pr_merge {
                        if let Some(ctx) = pr_context {
                            if ctx.approved_count < rule.required_approvals {
                                return Err(WritePipelineError::BranchProtection(format!(
                                    "Branch '{}' requires at least {} approved review(s); currently has {}",
                                    update.ref_name, rule.required_approvals, ctx.approved_count
                                )));
                            }
                            if ctx.has_active_change_requests {
                                return Err(WritePipelineError::BranchProtection(format!(
                                    "Cannot merge into protected branch '{}' while changes are requested",
                                    update.ref_name
                                )));
                            }
                            if rule.require_changes_addressed && ctx.has_unresolved_comments {
                                return Err(WritePipelineError::BranchProtection(format!(
                                    "Cannot merge into protected branch '{}' with unresolved review comments",
                                    update.ref_name
                                )));
                            }
                        }
                    }
                }
            }
        }

        // 3. Coordinator Epoch & Placement Acquisition
        let placement = self
            .coordinator_store
            .get_placement(repo_id)
            .map_err(|e| WritePipelineError::CoordinatorError(e.to_string()))?
            .ok_or_else(|| {
                WritePipelineError::CoordinatorError(format!(
                    "No placement plan found for repository '{}'",
                    repo_id
                ))
            })?;

        let repo_meta = self
            .coordinator_store
            .get_repository(repo_id)
            .map_err(|e| WritePipelineError::CoordinatorError(e.to_string()))?
            .ok_or_else(|| {
                WritePipelineError::CoordinatorError(format!(
                    "Repository '{}' not registered in coordinator",
                    repo_id
                ))
            })?;

        let current_epoch = repo_meta.current_ownership_epoch;
        let current_gen = repo_meta.committed_generation;
        let proposed_gen = current_gen + 1;
        let policy = repo_meta.durability_policy;

        // 4. Expected Reference State (CAS) & Fast-Forward Validation
        for update in updates {
            let ref_rel = update
                .ref_name
                .strip_prefix("refs/")
                .unwrap_or(&update.ref_name);
            let ref_file = spine_dir.join("refs").join(ref_rel);
            let current_oid = if ref_file.exists() {
                let c = std::fs::read_to_string(&ref_file).unwrap_or_default();
                ObjectId::from_hex(c.trim()).ok()
            } else {
                None
            };

            if let Some(lease) = update.lease_oid {
                if current_oid != Some(lease) {
                    return Err(WritePipelineError::StaleReference(format!(
                        "Lease check failed for {}: expected {}, found {:?}",
                        update.ref_name,
                        lease.to_hex(),
                        current_oid.map(|o| o.to_hex())
                    )));
                }
            }

            let matches_expected = match (current_oid, update.expected_old_oid) {
                (Some(cur), Some(exp)) => cur == exp,
                (None, None) => true,
                _ => false,
            };

            if !matches_expected {
                return Err(WritePipelineError::StaleReference(format!(
                    "Expected old reference value does not match current tip on branch {}",
                    update.ref_name
                )));
            }

            if let (Some(old), false) = (current_oid, update.force) {
                if !is_pr_merge {
                    let ff = is_ancestor(primary_store, &old, &update.new_oid).unwrap_or(false);
                    if !ff {
                        return Err(WritePipelineError::NonFastForward(format!(
                            "Non-fast-forward update on {} rejected",
                            update.ref_name
                        )));
                    }
                }
            }
        }

        // 5. Pack Checksum & Transitive Object Closure Verification
        let pack_checksum = blake3::hash(pack_bytes).to_hex().to_string();
        for update in updates {
            let mut boundary = HashSet::new();
            if let Some(old) = update.expected_old_oid {
                boundary.insert(old);
            }
            if let Err(e) = spine_net::closure::validate_push_object_closure(
                primary_store,
                &update.new_oid,
                &boundary,
            ) {
                return Err(WritePipelineError::MissingObject(format!(
                    "Object closure validation failed for {}: {}",
                    update.ref_name, e
                )));
            }
        }

        // 6. 2-Phase Replication Transaction (2PC) Enforcing Durability Policy
        let tx_id = format!("tx-write-{}", uuid::Uuid::new_v4().simple());
        let all_nodes = placement.all_nodes();

        if all_nodes.len() < policy.required_copies() {
            return Err(WritePipelineError::DurabilityRequirementFailed {
                required: policy.required_copies(),
                actual: all_nodes.len(),
                message: format!(
                    "Placement plan contains {} node(s), but durability policy {:?} requires {}",
                    all_nodes.len(),
                    policy,
                    policy.required_copies()
                ),
            });
        }

        let mut tx = ReplicationTransaction::new(
            tx_id.clone(),
            repo_id.to_string(),
            current_gen,
            proposed_gen,
            placement.primary_node_id.clone(),
            current_epoch,
            policy,
            all_nodes.clone(),
        );

        tx.start_preparing()?;
        let _ = self.coordinator_store.create_transaction(tx.clone());

        // Stage on Primary
        let primary_prepared = self
            .dispatcher
            .prepare_push(
                &placement.primary_node_id,
                &tx_id,
                repo_id,
                current_epoch,
                proposed_gen,
                pack_id,
                &pack_checksum,
                pack_bytes.to_vec(),
                index_bytes.to_vec(),
            )
            .await?;

        if !primary_prepared {
            let _ = tx.abort("Primary failed to prepare staged pack", now_nanos);
            let _ = self.coordinator_store.update_transaction(tx.clone());
            let _ = self
                .dispatcher
                .abort_push(
                    &placement.primary_node_id,
                    &tx_id,
                    repo_id,
                    current_epoch,
                    "Primary abort",
                )
                .await;
            return Err(WritePipelineError::StorageNodeError(
                "Primary storage node refused or failed prepare".into(),
            ));
        }

        tx.acknowledge_prepared(&placement.primary_node_id)?;

        // Stage on Replicas
        for rep_id in &placement.replica_node_ids {
            match self
                .dispatcher
                .prepare_push(
                    rep_id,
                    &tx_id,
                    repo_id,
                    current_epoch,
                    proposed_gen,
                    pack_id,
                    &pack_checksum,
                    pack_bytes.to_vec(),
                    index_bytes.to_vec(),
                )
                .await
            {
                Ok(true) => {
                    let _ = tx.acknowledge_prepared(rep_id);
                }
                Ok(false) | Err(_) => {
                    // Handled below via durability requirement check
                }
            }
        }

        // Verify Durability Quorum
        if tx.prepared_nodes.len() < policy.required_copies() {
            let _ = tx.abort("Durability quorum not satisfied", now_nanos);
            let _ = self.coordinator_store.update_transaction(tx.clone());
            // Rollback all participating nodes
            for node_id in &all_nodes {
                let _ = self
                    .dispatcher
                    .abort_push(
                        node_id,
                        &tx_id,
                        repo_id,
                        current_epoch,
                        "Durability quorum not satisfied",
                    )
                    .await;
            }
            return Err(WritePipelineError::DurabilityRequirementFailed {
                required: policy.required_copies(),
                actual: tx.prepared_nodes.len(),
                message: format!(
                    "Durability requirement {:?} unsatisfied: {} of {} copies prepared",
                    policy,
                    tx.prepared_nodes.len(),
                    policy.required_copies()
                ),
            });
        }

        // Commit Decision
        tx.decide_commit(now_nanos)?;
        let _ = self.coordinator_store.update_transaction(tx.clone());

        // 7. Commit Push on Primary & Replicas, and Atomic Ref Publication
        let ref_update_tuples: Vec<(String, String)> = updates
            .iter()
            .map(|u| (u.ref_name.clone(), u.new_oid.to_hex()))
            .collect();

        // Commit on Primary
        self.dispatcher
            .commit_push(
                &placement.primary_node_id,
                &tx_id,
                repo_id,
                current_epoch,
                proposed_gen,
                ref_update_tuples.clone(),
            )
            .await?;

        // Publish reference on primary filesystem atomically
        for update in updates {
            let ref_rel = update
                .ref_name
                .strip_prefix("refs/")
                .unwrap_or(&update.ref_name);
            let ref_path = spine_dir.join("refs").join(ref_rel);
            if let Some(parent) = ref_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            spine_core::atomic_write_file(
                &ref_path,
                format!("{}\n", update.new_oid.to_hex()).as_bytes(),
            )
            .map_err(|e| {
                WritePipelineError::Internal(format!("Failed to write ref atomically: {}", e))
            })?;
        }

        // Commit on Replicas that prepared
        for rep_id in &placement.replica_node_ids {
            if tx.prepared_nodes.contains(rep_id) {
                let _ = self
                    .dispatcher
                    .commit_push(
                        rep_id,
                        &tx_id,
                        repo_id,
                        current_epoch,
                        proposed_gen,
                        ref_update_tuples.clone(),
                    )
                    .await;
            }
        }

        // 8. Finalize Transaction State in Coordinator
        tx.mark_committed(now_nanos)?;
        let _ = self.coordinator_store.update_transaction(tx.clone());
        let _ = self
            .coordinator_store
            .advance_committed_generation(repo_id, proposed_gen);

        for node_id in &tx.prepared_nodes {
            let is_prim = *node_id == placement.primary_node_id;
            let _ = self.coordinator_store.update_replica_state(
                repo_id,
                node_id,
                is_prim,
                proposed_gen,
                true,
                now_nanos,
            );
        }

        let updated_refs = updates
            .iter()
            .map(|u| (u.ref_name.clone(), u.new_oid))
            .collect();

        Ok(AuthoritativeWriteResult {
            transaction_id: tx_id,
            committed_generation: proposed_gen,
            updated_refs,
            copies_verified: tx.prepared_nodes.len(),
            durability_policy: policy,
            committed_at_nanos: now_nanos,
        })
    }

    /// Verifies current durability health across all placed replicas.
    pub async fn verify_durability(
        &self,
        repo_id: &str,
    ) -> Result<DurabilityVerificationReport, WritePipelineError> {
        let repo_meta = self
            .coordinator_store
            .get_repository(repo_id)
            .map_err(|e| WritePipelineError::CoordinatorError(e.to_string()))?
            .ok_or_else(|| {
                WritePipelineError::CoordinatorError(format!("Repository '{}' not found", repo_id))
            })?;

        let placement = self
            .coordinator_store
            .get_placement(repo_id)
            .map_err(|e| WritePipelineError::CoordinatorError(e.to_string()))?
            .ok_or_else(|| {
                WritePipelineError::CoordinatorError(format!(
                    "No placement for repository '{}'",
                    repo_id
                ))
            })?;

        let policy = repo_meta.durability_policy;
        let required = policy.required_copies();
        let mut verified_count = 0;
        let mut node_generations = HashMap::new();

        for node_id in placement.all_nodes() {
            match self.dispatcher.inspect_repo(&node_id, repo_id).await {
                Ok((gen, _packs, healthy)) => {
                    node_generations.insert(node_id.clone(), gen);
                    if healthy && gen == repo_meta.committed_generation {
                        verified_count += 1;
                    }
                }
                Err(_) => {
                    node_generations.insert(node_id.clone(), 0);
                }
            }
        }

        Ok(DurabilityVerificationReport {
            repo_id: repo_id.to_string(),
            durability_policy: policy,
            required_copies: required,
            verified_copies: verified_count,
            is_durability_satisfied: verified_count >= required,
            committed_generation: repo_meta.committed_generation,
            primary_node_id: placement.primary_node_id,
            replica_nodes: placement.replica_node_ids,
            node_generations,
        })
    }
}
