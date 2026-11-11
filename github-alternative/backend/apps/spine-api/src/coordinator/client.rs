use crate::core::error::ForgeError;
use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::placement::PlacementPlan;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse, VolumeSnapshotManifest};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Clone)]
pub struct ClusterNodeClient {
    addr: String,
    timeout_duration: Duration,
}

impl ClusterNodeClient {
    pub fn new(addr: String) -> Self {
        Self {
            addr,
            timeout_duration: Duration::from_secs(15),
        }
    }

    pub async fn send_rpc(
        &self,
        req: &ClusterRpcRequest,
    ) -> Result<ClusterRpcResponse, ForgeError> {
        let connect_fut = TcpStream::connect(&self.addr);
        let mut stream = timeout(self.timeout_duration, connect_fut)
            .await
            .map_err(|_| {
                ForgeError::ServiceUnavailable(format!(
                    "Timed out connecting to cluster node at {}",
                    self.addr
                ))
            })?
            .map_err(|e| {
                ForgeError::ServiceUnavailable(format!(
                    "Failed to connect to cluster node at {}: {}",
                    self.addr, e
                ))
            })?;

        let req_bytes = req
            .to_bytes()
            .map_err(|e| ForgeError::Internal(format!("RPC serialization error: {}", e)))?;
        let len_buf = (req_bytes.len() as u32).to_be_bytes();

        stream
            .write_all(&len_buf)
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("RPC write error: {}", e)))?;
        stream
            .write_all(&req_bytes)
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("RPC write error: {}", e)))?;
        stream
            .flush()
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("RPC flush error: {}", e)))?;

        let mut resp_len_buf = [0u8; 4];
        stream
            .read_exact(&mut resp_len_buf)
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("RPC read length error: {}", e)))?;
        let resp_len = u32::from_be_bytes(resp_len_buf) as usize;

        let mut resp_body = vec![0u8; resp_len];
        stream
            .read_exact(&mut resp_body)
            .await
            .map_err(|e| ForgeError::ServiceUnavailable(format!("RPC read body error: {}", e)))?;

        let resp = ClusterRpcResponse::from_bytes(&resp_body)
            .map_err(|e| ForgeError::Internal(format!("RPC response parse error: {}", e)))?;

        Ok(resp)
    }

    pub async fn enter_maintenance(&self, fencing_generation: u64) -> Result<(), ForgeError> {
        let req = ClusterRpcRequest::EnterMaintenanceBarrier { fencing_generation };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::MaintenanceBarrierAck { success } if success => Ok(()),
            ClusterRpcResponse::MaintenanceBarrierAck { .. } => Err(ForgeError::Internal(
                "Enter maintenance barrier rejected".into(),
            )),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Enter maintenance failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn exit_maintenance(&self, fencing_generation: u64) -> Result<(), ForgeError> {
        let req = ClusterRpcRequest::ExitMaintenanceBarrier { fencing_generation };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::MaintenanceBarrierAck { success } if success => Ok(()),
            ClusterRpcResponse::MaintenanceBarrierAck { .. } => Err(ForgeError::Internal(
                "Exit maintenance barrier rejected".into(),
            )),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Exit maintenance failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn query_maintenance_status(&self) -> Result<(bool, u64), ForgeError> {
        let (in_maint, gen, _, _) = self.query_node_details().await?;
        Ok((in_maint, gen))
    }

    pub async fn query_node_details(
        &self,
    ) -> Result<(bool, u64, usize, Option<String>), ForgeError> {
        let req = ClusterRpcRequest::QueryMaintenanceStatus;
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::MaintenanceStatus {
                in_maintenance,
                fencing_generation,
                active_writers,
                node_id,
            } => Ok((in_maintenance, fencing_generation, active_writers, node_id)),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Query maintenance status failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn capture_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>), ForgeError> {
        let req = ClusterRpcRequest::CaptureVolumeSnapshot {
            snapshot_id: snapshot_id.to_string(),
            fencing_generation,
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::VolumeSnapshotCaptured {
                manifest,
                payload_bytes,
            } => Ok((manifest, payload_bytes)),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Capture snapshot failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn stage_restore(
        &self,
        manifest: VolumeSnapshotManifest,
        payload_bytes: Vec<u8>,
    ) -> Result<spine_cluster::protocol::ParticipantAcknowledgement, ForgeError> {
        let req = ClusterRpcRequest::StageVolumeRestore {
            snapshot_id: manifest.snapshot_id.clone(),
            fencing_generation: manifest.fencing_generation,
            manifest: manifest.clone(),
            payload_bytes,
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::VolumeRestoreStaged {
                success,
                acknowledgement,
                snapshot_id,
            } if success => {
                if let Some(ack) = acknowledgement {
                    Ok(ack)
                } else {
                    let canonical_res =
                        format!("{}_{}", manifest.participant_id, manifest.volume_name);
                    let rec_id = format!(
                        ".staged_volume_{}/.staged_restore_record.json",
                        manifest.snapshot_id
                    );
                    Ok(spine_cluster::protocol::ParticipantAcknowledgement {
                        restore_id: format!("rst_{}", snapshot_id),
                        backup_id: snapshot_id,
                        participant_id: manifest.participant_id,
                        volume_name: manifest.volume_name,
                        canonical_resource_id: canonical_res,
                        previous_generation: None,
                        target_generation: manifest.fencing_generation.to_string(),
                        staged_digest: manifest.sha256_checksum.clone(),
                        archive_payload_sha256: manifest.sha256_checksum.clone(),
                        staged_resource_digest: manifest.sha256_checksum,
                        staged_byte_length: manifest.total_bytes,
                        prepared_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                        prepared: true,
                        durable_staging_record_id: rec_id,
                        current_state: "Staged".to_string(),
                    })
                }
            }
            ClusterRpcResponse::VolumeRestoreStaged { .. } => {
                Err(ForgeError::Internal("Stage restore rejected".into()))
            }
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Stage restore failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn commit_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(), ForgeError> {
        let req = ClusterRpcRequest::CommitVolumeRestore {
            snapshot_id: snapshot_id.to_string(),
            fencing_generation,
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::VolumeRestoreCommitted { success, .. } if success => Ok(()),
            ClusterRpcResponse::VolumeRestoreCommitted { .. } => {
                Err(ForgeError::Internal("Commit restore rejected".into()))
            }
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Commit restore failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn rollback_restore(&self, snapshot_id: &str) -> Result<(), ForgeError> {
        let req = ClusterRpcRequest::RollbackVolumeRestore {
            snapshot_id: snapshot_id.to_string(),
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::VolumeRestoreRolledBack { .. } => Ok(()),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Rollback restore failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn finalize_restore(&self, snapshot_id: &str) -> Result<(), ForgeError> {
        let req = ClusterRpcRequest::FinalizeVolumeRestore {
            snapshot_id: snapshot_id.to_string(),
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::VolumeRestoreFinalized { .. } => Ok(()),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Finalize restore failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal("Unexpected RPC response".into())),
        }
    }

    pub async fn query_staged_restore(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<spine_cluster::protocol::ParticipantAcknowledgement>, ForgeError> {
        let req = ClusterRpcRequest::QueryStagedVolumeRestore {
            snapshot_id: snapshot_id.to_string(),
        };
        match self.send_rpc(&req).await? {
            ClusterRpcResponse::StagedVolumeRestoreStatus {
                prepared,
                acknowledgement,
                ..
            } => {
                if prepared {
                    Ok(acknowledgement)
                } else {
                    Ok(None)
                }
            }
            ClusterRpcResponse::Error(msg) => Err(ForgeError::Internal(format!(
                "Query staged restore failed: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal(
                "Unexpected RPC response for query staged restore".into(),
            )),
        }
    }
}

#[derive(Clone)]
pub struct CoordinatorClient {
    coordinator_addr: String,
    node_client: ClusterNodeClient,
}

impl CoordinatorClient {
    pub fn new(coordinator_addr: String) -> Self {
        let node_client = ClusterNodeClient::new(coordinator_addr.clone());
        Self {
            coordinator_addr,
            node_client,
        }
    }

    pub fn addr(&self) -> &str {
        &self.coordinator_addr
    }

    async fn send_rpc(&self, req: &ClusterRpcRequest) -> Result<ClusterRpcResponse, ForgeError> {
        self.node_client.send_rpc(req).await
    }

    pub async fn allocate_repository(
        &self,
        repo_id: &str,
        name: &str,
        owner: &str,
        durability_policy: DurabilityPolicy,
    ) -> Result<PlacementPlan, ForgeError> {
        let req = ClusterRpcRequest::AllocateRepository {
            repo_id: repo_id.to_string(),
            name: name.to_string(),
            owner: owner.to_string(),
            durability_policy,
        };

        match self.send_rpc(&req).await? {
            ClusterRpcResponse::AllocationResult { placement, .. } => Ok(placement),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::ServiceUnavailable(format!(
                "Coordinator allocation rejected: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal(
                "Unexpected coordinator response to AllocateRepository".into(),
            )),
        }
    }

    pub async fn get_placement(&self, repo_id: &str) -> Result<Option<PlacementPlan>, ForgeError> {
        let req = ClusterRpcRequest::GetPlacement {
            repo_id: repo_id.to_string(),
        };

        match self.send_rpc(&req).await? {
            ClusterRpcResponse::PlacementResult(plan) => Ok(plan),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::ServiceUnavailable(format!(
                "Coordinator GetPlacement error: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal(
                "Unexpected coordinator response to GetPlacement".into(),
            )),
        }
    }

    pub async fn delete_repository(&self, repo_id: &str) -> Result<bool, ForgeError> {
        let req = ClusterRpcRequest::DeleteRepository {
            repo_id: repo_id.to_string(),
        };

        match self.send_rpc(&req).await? {
            ClusterRpcResponse::DeleteResult { success, .. } => Ok(success),
            ClusterRpcResponse::Error(msg) => Err(ForgeError::ServiceUnavailable(format!(
                "Coordinator DeleteRepository error: {}",
                msg
            ))),
            _ => Err(ForgeError::Internal(
                "Unexpected coordinator response to DeleteRepository".into(),
            )),
        }
    }

    pub async fn check_health(&self) -> bool {
        let req = ClusterRpcRequest::GetPlacement {
            repo_id: "health-ping".to_string(),
        };
        self.send_rpc(&req).await.is_ok()
    }

    pub async fn enter_maintenance(&self, fencing_generation: u64) -> Result<(), ForgeError> {
        self.node_client.enter_maintenance(fencing_generation).await
    }

    pub async fn exit_maintenance(&self, fencing_generation: u64) -> Result<(), ForgeError> {
        self.node_client.exit_maintenance(fencing_generation).await
    }

    pub async fn query_maintenance_status(&self) -> Result<(bool, u64), ForgeError> {
        self.node_client.query_maintenance_status().await
    }

    pub async fn query_node_details(
        &self,
    ) -> Result<(bool, u64, usize, Option<String>), ForgeError> {
        self.node_client.query_node_details().await
    }

    pub async fn capture_snapshot(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(VolumeSnapshotManifest, Vec<u8>), ForgeError> {
        self.node_client
            .capture_snapshot(snapshot_id, fencing_generation)
            .await
    }

    pub async fn stage_restore(
        &self,
        manifest: VolumeSnapshotManifest,
        payload_bytes: Vec<u8>,
    ) -> Result<spine_cluster::protocol::ParticipantAcknowledgement, ForgeError> {
        self.node_client
            .stage_restore(manifest, payload_bytes)
            .await
    }

    pub async fn commit_restore(
        &self,
        snapshot_id: &str,
        fencing_generation: u64,
    ) -> Result<(), ForgeError> {
        self.node_client
            .commit_restore(snapshot_id, fencing_generation)
            .await
    }

    pub async fn rollback_restore(&self, snapshot_id: &str) -> Result<(), ForgeError> {
        self.node_client.rollback_restore(snapshot_id).await
    }

    pub async fn finalize_restore(&self, snapshot_id: &str) -> Result<(), ForgeError> {
        self.node_client.finalize_restore(snapshot_id).await
    }

    pub async fn query_staged_restore(
        &self,
        snapshot_id: &str,
    ) -> Result<Option<spine_cluster::protocol::ParticipantAcknowledgement>, ForgeError> {
        self.node_client.query_staged_restore(snapshot_id).await
    }
}
