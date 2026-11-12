use anyhow::{Context, Result};
use spine_cluster::failover::FailoverCoordinator;
use spine_cluster::fencing::OwnershipEpoch;
use spine_cluster::heartbeat::{
    evaluate_node_health, DEFAULT_OFFLINE_TIMEOUT_NANOS, DEFAULT_SUSPECT_TIMEOUT_NANOS,
};
use spine_cluster::node::{FailureDomain, NodeStatus, StorageNodeInfo};
use spine_cluster::placement::PlacementScheduler;
use spine_cluster::protocol::{
    ClusterRpcRequest, ClusterRpcResponse, MAX_CLUSTER_RPC_FRAME_BYTES,
    MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES,
};
use spine_cluster::store::{CoordinatorStore, RepositoryMeta};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};

pub struct CoordinatorServer {
    cluster_id: String,
    listen_addr: String,
    store: Arc<dyn CoordinatorStore>,
    running: Arc<AtomicBool>,
}

impl CoordinatorServer {
    pub fn new(cluster_id: String, listen_addr: String, store: Arc<dyn CoordinatorStore>) -> Self {
        Self {
            cluster_id,
            listen_addr,
            store,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.listen_addr)
            .await
            .with_context(|| format!("Binding coordinator to {}", self.listen_addr))?;

        println!(
            "Coordinator '{}' listening on {}",
            self.cluster_id, self.listen_addr
        );

        // Spawn background health monitoring and failover task
        let store = Arc::clone(&self.store);
        let running = Arc::clone(&self.running);

        tokio::spawn(async move {
            while running.load(Ordering::SeqCst) {
                let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                if let Ok(nodes) = store.list_nodes() {
                    for node in &nodes {
                        let new_status = evaluate_node_health(
                            node.last_heartbeat_nanos,
                            now,
                            node.status,
                            DEFAULT_SUSPECT_TIMEOUT_NANOS,
                            DEFAULT_OFFLINE_TIMEOUT_NANOS,
                        );

                        if new_status != node.status {
                            let _ = store.update_node_status(&node.node_id, new_status);

                            // If primary went offline, check if any repositories need failover!
                            if new_status == NodeStatus::Offline {
                                if let Ok(repos) = store.list_repositories() {
                                    for repo in repos {
                                        if let Ok(Some(placement)) =
                                            store.get_placement(&repo.repo_id)
                                        {
                                            if placement.primary_node_id == node.node_id {
                                                if let Ok(replicas) =
                                                    store.get_replicas(&repo.repo_id)
                                                {
                                                    if let Ok(fresh_nodes) = store.list_nodes() {
                                                        if let Ok(promo) = FailoverCoordinator::evaluate_and_promote(
                                                            &repo.repo_id,
                                                            &node.node_id,
                                                            OwnershipEpoch(repo.current_ownership_epoch),
                                                            repo.committed_generation,
                                                            &replicas,
                                                            &fresh_nodes,
                                                        ) {
                                                            let _ = store.update_ownership_epoch(
                                                                &repo.repo_id,
                                                                promo.new_ownership_epoch,
                                                                &promo.new_primary_id,
                                                            );
                                                            println!(
                                                                "FAILOVER: Repository '{}' promoted new primary '{}' with epoch {}",
                                                                repo.repo_id,
                                                                promo.new_primary_id,
                                                                promo.new_ownership_epoch
                                                            );
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                sleep(Duration::from_millis(500)).await;
            }
        });

        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok((stream, _peer)) => {
                            let store = Arc::clone(&self.store);
                            let cluster_id = self.cluster_id.clone();
                            tokio::spawn(async move {
                                let _ = handle_coordinator_connection(stream, store, cluster_id).await;
                            });
                        }
                        Err(e) => {
                            eprintln!("Coordinator accept error: {}", e);
                        }
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(100)) => {
                    if !self.running.load(Ordering::SeqCst) {
                        break;
                    }
                }
            }
        }

        println!("Coordinator shutdown complete.");
        Ok(())
    }
}

async fn handle_coordinator_connection(
    mut stream: TcpStream,
    store: Arc<dyn CoordinatorStore>,
    cluster_id: String,
) -> Result<()> {
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len == 0 || len > MAX_CLUSTER_RPC_FRAME_BYTES {
        anyhow::bail!(
            "Cluster RPC frame size {} bytes exceeds bound {}",
            len,
            MAX_CLUSTER_RPC_FRAME_BYTES
        );
    }

    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;

    let req = ClusterRpcRequest::from_bytes(&body)?;
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let resp = match req {
        ClusterRpcRequest::Enroll(enroll_req) => match enroll_req.verify_signature(&cluster_id) {
            Ok(_) => match store.get_token(&enroll_req.token) {
                Ok(Some(token)) => match token.validate(&cluster_id, now) {
                    Ok(_) => {
                        let _ = store.consume_token(&enroll_req.token);
                        let node_id = format!("node-{}", &enroll_req.public_key_hex[..8]);
                        let fingerprint = format!("fp:{}", &enroll_req.public_key_hex[..12]);

                        let mut info = StorageNodeInfo::new(
                            node_id.clone(),
                            enroll_req.public_key_hex.clone(),
                            fingerprint,
                            enroll_req.listen_addr,
                            enroll_req.storage_root,
                            FailureDomain {
                                region: enroll_req.region,
                                zone: enroll_req.zone,
                                host: enroll_req.host,
                            },
                            enroll_req.total_capacity_bytes,
                            now,
                        );
                        info.status = NodeStatus::Healthy;

                        if let Err(e) = store.register_node(info) {
                            ClusterRpcResponse::Error(e.to_string())
                        } else {
                            ClusterRpcResponse::EnrollSuccess(
                                spine_cluster::enrollment::EnrollmentResponse {
                                    node_id,
                                    cluster_id,
                                    enrolled_at_nanos: now,
                                },
                            )
                        }
                    }
                    Err(e) => ClusterRpcResponse::Error(e.to_string()),
                },
                Ok(None) => ClusterRpcResponse::Error("Enrollment token not found".into()),
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::Heartbeat(hb) => {
            if let Err(e) = store.record_heartbeat(hb) {
                ClusterRpcResponse::Error(e.to_string())
            } else {
                ClusterRpcResponse::HeartbeatAck {
                    status: NodeStatus::Healthy,
                }
            }
        }
        ClusterRpcRequest::GetPlacement { repo_id } => match store.get_placement(&repo_id) {
            Ok(p) => ClusterRpcResponse::PlacementResult(p),
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::AllocateRepository {
            repo_id,
            name,
            owner,
            durability_policy,
        } => match store.get_placement(&repo_id) {
            Ok(Some(existing_plan)) => ClusterRpcResponse::AllocationResult {
                repo_id,
                placement: existing_plan,
            },
            _ => match store.list_nodes() {
                Ok(nodes) => {
                    match PlacementScheduler::schedule_placement(
                        &repo_id,
                        durability_policy,
                        &nodes,
                        0,
                        1,
                    ) {
                        Ok(plan) => {
                            let meta = RepositoryMeta {
                                repo_id: repo_id.clone(),
                                name,
                                owner,
                                durability_policy,
                                committed_generation: 0,
                                current_ownership_epoch: 1,
                                storage_quota_bytes: 10 * 1024 * 1024 * 1024,
                                state: "HEALTHY".to_string(),
                                created_at_nanos: now,
                                updated_at_nanos: now,
                            };
                            let _ = store.create_repository(meta);
                            let _ = store.set_placement(plan.clone());
                            let _ = store.update_replica_state(
                                &repo_id,
                                &plan.primary_node_id,
                                true,
                                0,
                                true,
                                now,
                            );
                            for rep in &plan.replica_node_ids {
                                let _ =
                                    store.update_replica_state(&repo_id, rep, false, 0, true, now);
                            }
                            let _ =
                                store.update_ownership_epoch(&repo_id, 1, &plan.primary_node_id);
                            ClusterRpcResponse::AllocationResult {
                                repo_id,
                                placement: plan,
                            }
                        }
                        Err(e) => ClusterRpcResponse::Error(format!("Scheduling failed: {}", e)),
                    }
                }
                Err(e) => ClusterRpcResponse::Error(format!("Listing nodes failed: {}", e)),
            },
        },
        ClusterRpcRequest::DeleteRepository { repo_id } => {
            let tombstone = spine_cluster::gc::TombstoneRecord::new(
                format!("tb-{}", uuid::Uuid::new_v4()),
                repo_id.clone(),
                "ALL_PACKS".to_string(),
                None,
                0,
                86400,
                now,
            );
            match store.create_tombstone(tombstone) {
                Ok(_) => ClusterRpcResponse::DeleteResult {
                    repo_id,
                    success: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::DrainNode { node_id } => {
            match store.update_node_status(&node_id, NodeStatus::Draining) {
                Ok(_) => ClusterRpcResponse::NodeStatusUpdated {
                    node_id,
                    new_status: NodeStatus::Draining,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::QuarantineNode { node_id } => {
            match store.update_node_status(&node_id, NodeStatus::Quarantined) {
                Ok(_) => ClusterRpcResponse::NodeStatusUpdated {
                    node_id,
                    new_status: NodeStatus::Quarantined,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::RetireNode { node_id } => {
            match store.update_node_status(&node_id, NodeStatus::Retired) {
                Ok(_) => ClusterRpcResponse::NodeStatusUpdated {
                    node_id,
                    new_status: NodeStatus::Retired,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation } => {
            match store.enter_maintenance(fencing_generation) {
                Ok(_) => ClusterRpcResponse::MaintenanceBarrierAck { success: true },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::ExitMaintenanceBarrier { fencing_generation } => {
            match store.exit_maintenance(fencing_generation) {
                Ok(_) => ClusterRpcResponse::MaintenanceBarrierAck { success: true },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::QueryMaintenanceStatus => {
            let (in_maintenance, fencing_generation, active_writers) =
                store.get_maintenance_status();
            ClusterRpcResponse::MaintenanceStatus {
                in_maintenance,
                fencing_generation,
                active_writers,
                node_id: Some("coordinator".to_string()),
            }
        }
        ClusterRpcRequest::CaptureVolumeSnapshot {
            snapshot_id,
            fencing_generation,
        } => match store.capture_snapshot(&snapshot_id, fencing_generation) {
            Ok((manifest, payload_bytes)) => {
                if payload_bytes.len() > MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES {
                    ClusterRpcResponse::Error(format!(
                        "Captured coordinator snapshot payload ({} bytes) exceeds MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES ({} bytes)",
                        payload_bytes.len(),
                        MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES
                    ))
                } else {
                    ClusterRpcResponse::VolumeSnapshotCaptured {
                        manifest,
                        payload_bytes,
                    }
                }
            }
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::StageVolumeRestore {
            snapshot_id,
            fencing_generation,
            manifest,
            payload_bytes,
        } => {
            if payload_bytes.len() > MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES {
                ClusterRpcResponse::Error(format!(
                    "Staged coordinator snapshot payload ({} bytes) exceeds MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES ({} bytes)",
                    payload_bytes.len(),
                    MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES
                ))
            } else {
                match store.stage_restore_with_manifest(
                    &snapshot_id,
                    fencing_generation,
                    Some(&manifest),
                    &payload_bytes,
                ) {
                    Ok(ack) => ClusterRpcResponse::VolumeRestoreStaged {
                        snapshot_id,
                        success: true,
                        acknowledgement: Some(ack),
                    },
                    Err(e) => ClusterRpcResponse::Error(e.to_string()),
                }
            }
        }
        ClusterRpcRequest::CommitVolumeRestore {
            snapshot_id,
            fencing_generation,
        } => match store.commit_restore(&snapshot_id, fencing_generation) {
            Ok(_) => ClusterRpcResponse::VolumeRestoreCommitted {
                snapshot_id,
                success: true,
            },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::RollbackVolumeRestore { snapshot_id } => {
            match store.rollback_restore(&snapshot_id) {
                Ok(_) => ClusterRpcResponse::VolumeRestoreRolledBack {
                    snapshot_id,
                    success: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::FinalizeVolumeRestore { snapshot_id } => {
            match store.finalize_restore(&snapshot_id) {
                Ok(_) => ClusterRpcResponse::VolumeRestoreFinalized {
                    snapshot_id,
                    success: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::QueryStagedVolumeRestore { snapshot_id } => {
            match store.query_staged_restore(&snapshot_id) {
                Ok(Some(ack)) => ClusterRpcResponse::StagedVolumeRestoreStatus {
                    snapshot_id,
                    prepared: ack.prepared,
                    acknowledgement: Some(ack),
                },
                Ok(None) => ClusterRpcResponse::StagedVolumeRestoreStatus {
                    snapshot_id,
                    prepared: false,
                    acknowledgement: None,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        _ => ClusterRpcResponse::Error("Unsupported coordinator request".into()),
    };

    let resp_bytes = resp.to_bytes()?;
    if resp_bytes.len() > MAX_CLUSTER_RPC_FRAME_BYTES {
        anyhow::bail!(
            "Coordinator RPC response frame ({} bytes) exceeds MAX_CLUSTER_RPC_FRAME_BYTES ({} bytes)",
            resp_bytes.len(),
            MAX_CLUSTER_RPC_FRAME_BYTES
        );
    }
    let resp_len = (resp_bytes.len() as u32).to_be_bytes();
    stream.write_all(&resp_len).await?;
    stream.write_all(&resp_bytes).await?;
    stream.flush().await?;

    Ok(())
}
