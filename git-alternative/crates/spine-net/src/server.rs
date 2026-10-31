//! Spine Wire Protocol server session handler.

use crate::auth::verify_auth_signature;
use crate::negotiation::compute_missing_objects;
use crate::pack_transfer::{create_pack_file, receive_and_ingest_pack, send_pack_file};
use spine_core::{HashAlgorithm, ObjectId};
use spine_history::is_ancestor;
use spine_object_store::{lock::RepoLock, CompositeObjectStore};
use spine_protocol::{
    frame::{Frame, FrameType, ProtocolError},
    messages::{
        AuthChallenge, AuthResponse, AuthResult, ErrorMessage, FetchNegotiate, FetchPlan,
        HelloAckMessage, HelloMessage, ProtocolMessage, PushBegin, RefAdvertise, RefUpdateReq,
        RefUpdateRes, RefUpdateStatus, RepoInfo, RepoOpen,
    },
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::net::TcpStream;

#[derive(Error, Debug)]
pub enum ServerError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Protocol error: {0}")]
    Protocol(#[from] ProtocolError),
    #[error("Transfer error: {0}")]
    Transfer(#[from] crate::pack_transfer::TransferError),
    #[error("Negotiation error: {0}")]
    Negotiation(#[from] crate::negotiation::NegotiationError),
    #[error("Repository error: {0}")]
    Repository(String),
}

/// Discovers all local branches and tags in a `.spine` repository.
pub fn scan_repository_refs(spine_dir: &Path) -> BTreeMap<String, ObjectId> {
    let mut refs = BTreeMap::new();
    let heads_dir = spine_dir.join("refs").join("heads");
    if heads_dir.exists() {
        if let Ok(entries) = fs::read_dir(heads_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(oid) = ObjectId::from_hex(content.trim()) {
                        refs.insert(format!("refs/heads/{}", name), oid);
                    }
                }
            }
        }
    }

    let tags_dir = spine_dir.join("refs").join("tags");
    if tags_dir.exists() {
        if let Ok(entries) = fs::read_dir(tags_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    if let Ok(oid) = ObjectId::from_hex(content.trim()) {
                        refs.insert(format!("refs/tags/{}", name), oid);
                    }
                }
            }
        }
    }

    refs
}

/// Executes a single client session from connection to termination.
pub async fn handle_server_session(
    mut stream: TcpStream,
    repositories_root: PathBuf,
    secret_key: [u8; 32],
) -> Result<(), ServerError> {
    // 1. Handshake HELLO
    let hello_frame = Frame::read_from_async(&mut stream).await?;
    let _hello = HelloMessage::from_frame(&hello_frame)?;

    let ack = HelloAckMessage {
        server_version: "0.1.0".to_string(),
        selected_version: 1,
        capabilities: vec![
            "pack_zstd".to_string(),
            "ed25519_auth".to_string(),
            "cas_push".to_string(),
        ],
        server_id: "spine-daemon-node-01".to_string(),
    };
    ack.to_frame()?.write_to_async(&mut stream).await?;

    // 2. Authentication challenge
    let mut nonce = [0u8; 32];
    let uuid_bytes = uuid::Uuid::new_v4();
    nonce[0..16].copy_from_slice(uuid_bytes.as_bytes());
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let challenge = AuthChallenge {
        nonce,
        timestamp_ms: now_ms,
    };
    challenge.to_frame()?.write_to_async(&mut stream).await?;

    let auth_resp_frame = Frame::read_from_async(&mut stream).await?;
    let auth_resp = AuthResponse::from_frame(&auth_resp_frame)?;

    let is_authed = verify_auth_signature(
        &secret_key,
        &auth_resp.user_id,
        &nonce,
        now_ms,
        &auth_resp.signature,
    )
    .unwrap_or(false);

    let auth_result = AuthResult {
        authenticated: is_authed,
        user: auth_resp.user_id.clone(),
        role: if is_authed {
            "contributor".to_string()
        } else {
            "none".to_string()
        },
        message: if is_authed {
            "Authentication successful".to_string()
        } else {
            "Invalid credentials".to_string()
        },
    };
    auth_result.to_frame()?.write_to_async(&mut stream).await?;

    if !is_authed {
        return Ok(());
    }

    // 3. Repository selection
    let repo_open_frame = Frame::read_from_async(&mut stream).await?;
    let repo_open = RepoOpen::from_frame(&repo_open_frame)?;

    let repo_dir = repositories_root.join(&repo_open.path);
    let spine_dir = if repo_dir.join(".spine").exists() {
        repo_dir.join(".spine")
    } else if repo_dir.exists() && repo_dir.join("objects").exists() {
        repo_dir // Bare repository
    } else {
        let err = ErrorMessage {
            code: 404,
            category: "not_found".to_string(),
            message: format!("Repository not found: {}", repo_open.path),
        };
        err.to_frame()?.write_to_async(&mut stream).await?;
        return Ok(());
    };

    let store = match CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3) {
        Ok(s) => s,
        Err(e) => {
            let err = ErrorMessage {
                code: 500,
                category: "store_error".to_string(),
                message: e.to_string(),
            };
            err.to_frame()?.write_to_async(&mut stream).await?;
            return Ok(());
        }
    };

    let refs = scan_repository_refs(&spine_dir);
    let default_branch = "main".to_string();
    let head_oid = refs.get("refs/heads/main").copied();

    let repo_info = RepoInfo {
        algorithm: store.algorithm(),
        default_branch,
        head_oid,
        storage_format: "v2".to_string(),
    };
    repo_info.to_frame()?.write_to_async(&mut stream).await?;

    // 4. Reference advertisement
    let ref_adv = RefAdvertise { refs };
    ref_adv.to_frame()?.write_to_async(&mut stream).await?;

    // 5. Command loop
    loop {
        let frame = match Frame::read_from_async(&mut stream).await {
            Ok(f) => f,
            Err(ProtocolError::UnexpectedEof) => break, // Clean disconnect
            Err(e) => return Err(ServerError::Protocol(e)),
        };

        match frame.header.frame_type {
            FrameType::FetchNegotiate => {
                let req = FetchNegotiate::from_frame(&frame)?;
                let missing_oids = compute_missing_objects(&store, &req.want, &req.have)?;

                let plan = FetchPlan {
                    objects_to_send: missing_oids.len() as u64,
                    estimated_bytes: (missing_oids.len() as u64) * 512,
                    already_up_to_date: missing_oids.is_empty(),
                };
                plan.to_frame()?.write_to_async(&mut stream).await?;

                if !missing_oids.is_empty() {
                    let tmp_pack = std::env::temp_dir()
                        .join(format!("spine-server-send-{}.spk", uuid::Uuid::new_v4()));
                    let (checksum, count) = create_pack_file(&store, &missing_oids, &tmp_pack)?;
                    send_pack_file(&tmp_pack, checksum, count, &mut stream).await?;
                    let _ = fs::remove_file(&tmp_pack);
                }
            }
            FrameType::PushBegin => {
                let _push_begin = PushBegin::from_frame(&frame)?;
                receive_and_ingest_pack(&mut stream, &spine_dir, &store).await?;
            }
            FrameType::RefUpdateReq => {
                let req = RefUpdateReq::from_frame(&frame)?;
                let _lock = RepoLock::acquire(&spine_dir, "push_ref_update").map_err(|e| {
                    ServerError::Repository(format!("Could not acquire repo lock: {}", e))
                })?;

                let mut results = Vec::new();
                let mut overall_success = true;

                for update in req.updates {
                    let ref_rel_path = if let Some(p) = update.ref_name.strip_prefix("refs/") {
                        p
                    } else {
                        &update.ref_name
                    };
                    let ref_file = spine_dir.join("refs").join(ref_rel_path);

                    let current_oid = if ref_file.exists() {
                        let content = fs::read_to_string(&ref_file).unwrap_or_default();
                        ObjectId::from_hex(content.trim()).ok()
                    } else {
                        None
                    };

                    // Check CAS condition
                    let matches_expected = match (current_oid, update.expected_old_oid) {
                        (Some(cur), Some(exp)) => cur == exp,
                        (None, None) => true,
                        _ => false,
                    };

                    if !matches_expected {
                        overall_success = false;
                        results.push(RefUpdateStatus {
                            ref_name: update.ref_name.clone(),
                            status: "stale_ref".to_string(),
                            current_oid,
                            message: Some(
                                "Expected reference tip does not match current tip".to_string(),
                            ),
                        });
                        continue;
                    }

                    // Check fast-forward reachability
                    if let (Some(old), false) = (current_oid, update.force) {
                        let is_ff = is_ancestor(&store, &old, &update.new_oid).unwrap_or(false);
                        if !is_ff {
                            overall_success = false;
                            results.push(RefUpdateStatus {
                                ref_name: update.ref_name.clone(),
                                status: "non_fast_forward".to_string(),
                                current_oid,
                                message: Some(
                                    "Non-fast-forward update rejected; use force to override"
                                        .to_string(),
                                ),
                            });
                            continue;
                        }
                    }

                    // Apply update atomically
                    if let Some(parent) = ref_file.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    spine_core::atomic_write_file(
                        &ref_file,
                        format!("{}\n", update.new_oid.to_hex()).as_bytes(),
                    )?;

                    results.push(RefUpdateStatus {
                        ref_name: update.ref_name,
                        status: "ok".to_string(),
                        current_oid: Some(update.new_oid),
                        message: None,
                    });
                }

                let res = RefUpdateRes {
                    success: overall_success,
                    results,
                };
                res.to_frame()?.write_to_async(&mut stream).await?;
            }
            FrameType::Ping => {
                let pong = spine_protocol::messages::PongMessage { token: 1 };
                pong.to_frame()?.write_to_async(&mut stream).await?;
            }
            _ => {}
        }
    }

    Ok(())
}
