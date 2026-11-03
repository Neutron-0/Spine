use crate::core::error::ForgeError;
use spine_cluster::durability::DurabilityPolicy;
use spine_cluster::placement::PlacementPlan;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;

#[derive(Clone)]
pub struct CoordinatorClient {
    coordinator_addr: String,
    timeout_duration: Duration,
}

impl CoordinatorClient {
    pub fn new(coordinator_addr: String) -> Self {
        Self {
            coordinator_addr,
            timeout_duration: Duration::from_secs(5),
        }
    }

    async fn send_rpc(&self, req: &ClusterRpcRequest) -> Result<ClusterRpcResponse, ForgeError> {
        let connect_fut = TcpStream::connect(&self.coordinator_addr);
        let mut stream = timeout(self.timeout_duration, connect_fut)
            .await
            .map_err(|_| {
                ForgeError::ServiceUnavailable(format!(
                    "Timed out connecting to coordinator at {}",
                    self.coordinator_addr
                ))
            })?
            .map_err(|e| {
                ForgeError::ServiceUnavailable(format!(
                    "Failed to connect to coordinator at {}: {}",
                    self.coordinator_addr, e
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
}
