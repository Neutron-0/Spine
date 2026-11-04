use crate::backend::StorageBackend;
use anyhow::{Context, Result};
use spine_cluster::heartbeat::NodeHeartbeat;
use spine_cluster::node::NodeStatus;
use spine_cluster::protocol::{ClusterRpcRequest, ClusterRpcResponse};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::time::{sleep, Duration};

pub struct StorageNodeServer {
    node_id: String,
    listen_addr: String,
    coordinator_addr: Option<String>,
    backend: Arc<dyn StorageBackend>,
    running: Arc<AtomicBool>,
}

impl StorageNodeServer {
    pub fn new(
        node_id: String,
        listen_addr: String,
        coordinator_addr: Option<String>,
        backend: Arc<dyn StorageBackend>,
    ) -> Self {
        Self {
            node_id,
            listen_addr,
            coordinator_addr,
            backend,
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    pub async fn run(&self) -> Result<()> {
        let listener = TcpListener::bind(&self.listen_addr)
            .await
            .with_context(|| format!("Binding to {}", self.listen_addr))?;

        println!(
            "Storage node '{}' listening on {}",
            self.node_id, self.listen_addr
        );

        // Spawn heartbeat task if coordinator configured
        if let Some(coord_addr) = &self.coordinator_addr {
            let node_id = self.node_id.clone();
            let backend = Arc::clone(&self.backend);
            let coord = coord_addr.clone();
            let running = Arc::clone(&self.running);

            tokio::spawn(async move {
                let mut gen = 1u64;
                while running.load(Ordering::SeqCst) {
                    if let Ok(cap) = backend.get_capacity() {
                        let hb = NodeHeartbeat::new(
                            node_id.clone(),
                            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                            gen,
                            cap,
                            0,
                            0,
                            0,
                            NodeStatus::Healthy,
                        );
                        let _ = send_rpc_request(&coord, &ClusterRpcRequest::Heartbeat(hb)).await;
                    }
                    gen += 1;
                    sleep(Duration::from_secs(2)).await;
                }
            });
        }

        while self.running.load(Ordering::SeqCst) {
            tokio::select! {
                res = listener.accept() => {
                    match res {
                        Ok((stream, _peer)) => {
                            let backend = Arc::clone(&self.backend);
                            let node_id = self.node_id.clone();
                            tokio::spawn(async move {
                                let _ = handle_connection(stream, backend, node_id).await;
                            });
                        }
                        Err(e) => {
                            eprintln!("Accept error: {}", e);
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

        println!("Storage node '{}' shutdown complete.", self.node_id);
        Ok(())
    }
}

async fn handle_connection(
    mut stream: TcpStream,
    backend: Arc<dyn StorageBackend>,
    _node_id: String,
) -> Result<()> {
    // Read length-delimited frame: 4 bytes big-endian length + JSON
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let len = u32::from_be_bytes(len_buf) as usize;

    let mut body = vec![0u8; len];
    stream.read_exact(&mut body).await?;

    let req = ClusterRpcRequest::from_bytes(&body)?;
    let resp = match req {
        ClusterRpcRequest::PreparePush {
            transaction_id,
            repo_id,
            ownership_epoch,
            proposed_generation: _,
            pack_id,
            pack_checksum,
            pack_bytes,
            index_bytes,
        } => {
            match backend.stage_pack(
                &transaction_id,
                &repo_id,
                ownership_epoch,
                &pack_id,
                &pack_checksum,
                &pack_bytes,
                &index_bytes,
            ) {
                Ok(_) => ClusterRpcResponse::PrepareAck {
                    transaction_id,
                    prepared: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::CommitPush {
            transaction_id,
            repo_id,
            ownership_epoch,
            new_generation,
        } => {
            match backend.commit_pack(&transaction_id, &repo_id, ownership_epoch, new_generation) {
                Ok(_) => ClusterRpcResponse::CommitAck {
                    transaction_id,
                    committed_generation: new_generation,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::AbortPush {
            transaction_id,
            repo_id: _,
            ownership_epoch: _,
            reason: _,
        } => match backend.abort_pack(&transaction_id) {
            Ok(_) => ClusterRpcResponse::AbortAck { transaction_id },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::ReplicatePack {
            repo_id,
            pack_id,
            pack_checksum,
            pack_bytes,
            index_bytes,
        } => {
            match backend.ingest_replicated_pack(
                &repo_id,
                &pack_id,
                &pack_checksum,
                &pack_bytes,
                &index_bytes,
            ) {
                Ok(_) => ClusterRpcResponse::ReplicateAck {
                    pack_id,
                    verified: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::InspectRepository { repo_id } => {
            let gen = backend.get_generation(&repo_id).unwrap_or(0);
            let packs = backend.list_packs(&repo_id).unwrap_or_default();
            let healthy = backend.verify_repository(&repo_id).unwrap_or(false);
            ClusterRpcResponse::InspectResult {
                repo_id,
                generation: gen,
                packs,
                is_healthy: healthy,
            }
        }
        ClusterRpcRequest::RepairPack {
            repo_id,
            pack_id,
            pack_checksum,
            pack_bytes,
            index_bytes,
        } => {
            match backend.ingest_replicated_pack(
                &repo_id,
                &pack_id,
                &pack_checksum,
                &pack_bytes,
                &index_bytes,
            ) {
                Ok(_) => ClusterRpcResponse::RepairAck {
                    pack_id,
                    success: true,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        ClusterRpcRequest::StoreChunk {
            chunk_hash,
            chunk_bytes,
        } => match backend.store_chunk(&chunk_hash, &chunk_bytes) {
            Ok(_) => ClusterRpcResponse::StoreChunkAck {
                chunk_hash,
                success: true,
            },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::GetChunk { chunk_hash } => match backend.get_chunk(&chunk_hash) {
            Ok(bytes) => ClusterRpcResponse::GetChunkAck {
                chunk_hash,
                chunk_bytes: bytes,
            },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::StoreManifest {
            manifest_id,
            manifest_bytes,
        } => match backend.store_manifest(&manifest_id, &manifest_bytes) {
            Ok(_) => ClusterRpcResponse::StoreManifestAck {
                manifest_id,
                success: true,
            },
            Err(e) => ClusterRpcResponse::Error(e.to_string()),
        },
        ClusterRpcRequest::GetManifest { manifest_id } => {
            match backend.get_manifest(&manifest_id) {
                Ok(bytes) => ClusterRpcResponse::GetManifestAck {
                    manifest_id,
                    manifest_bytes: bytes,
                },
                Err(e) => ClusterRpcResponse::Error(e.to_string()),
            }
        }
        _ => ClusterRpcResponse::Error("Unhandled storage node request".into()),
    };

    let resp_bytes = resp.to_bytes()?;
    let resp_len = (resp_bytes.len() as u32).to_be_bytes();
    stream.write_all(&resp_len).await?;
    stream.write_all(&resp_bytes).await?;
    stream.flush().await?;

    Ok(())
}

pub async fn send_rpc_request(addr: &str, req: &ClusterRpcRequest) -> Result<ClusterRpcResponse> {
    let mut stream = TcpStream::connect(addr)
        .await
        .with_context(|| format!("Connecting to {}", addr))?;

    let req_bytes = req.to_bytes()?;
    let req_len = (req_bytes.len() as u32).to_be_bytes();
    stream.write_all(&req_len).await?;
    stream.write_all(&req_bytes).await?;
    stream.flush().await?;

    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf).await?;
    let resp_len = u32::from_be_bytes(len_buf) as usize;

    let mut resp_body = vec![0u8; resp_len];
    stream.read_exact(&mut resp_body).await?;

    let resp = ClusterRpcResponse::from_bytes(&resp_body)?;
    Ok(resp)
}
