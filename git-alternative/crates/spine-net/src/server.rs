//! Spine Wire Protocol server session handler with state machine validation,
//! cryptographic authentication, RBAC, lease-based force push, and object closure verification.

use crate::auth::{
    verify_asymmetric_auth, verify_auth_signature, verify_symmetric_auth_signature, AuthError,
    AuthTranscript, BoundedReplayCache, KeyRegistry, Role, DEFAULT_SECRET_KEY,
};
use crate::closure::validate_push_object_closure;
use crate::negotiation::compute_missing_objects;
use crate::pack_transfer::{create_pack_file, receive_and_ingest_pack, send_pack_file};
use rand::RngCore;
use spine_core::{HashAlgorithm, ObjectId};
use spine_history::is_ancestor;
use spine_object_store::{lock::RepoLock, CompositeObjectStore};
use spine_protocol::{
    frame::{Frame, FrameType, ProtocolError, SessionState},
    messages::{
        AuthChallenge, AuthResponse, AuthResult, ErrorMessage, FetchNegotiate, FetchPlan,
        HelloAckMessage, HelloMessage, ProtocolMessage, PushBegin, RefAdvertise, RefUpdateReq,
        RefUpdateRes, RefUpdateStatus, RepoInfo, RepoOpen,
    },
};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use thiserror::Error;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::net::TcpStream;

const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

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
    #[error("Authentication error: {0}")]
    Auth(#[from] AuthError),
    #[error("Timeout: operation timed out after {0:?}")]
    Timeout(Duration),
}

/// Server session configuration parameters.
#[derive(Clone)]
pub struct ServerSessionConfig {
    pub secret_key: [u8; 32],
    pub key_registry: Arc<KeyRegistry>,
    pub replay_cache: Arc<BoundedReplayCache>,
    pub server_id: String,
    pub allow_symmetric_auth: bool,
    pub require_tls: bool,
}

impl Default for ServerSessionConfig {
    fn default() -> Self {
        Self {
            secret_key: *DEFAULT_SECRET_KEY,
            key_registry: Arc::new(KeyRegistry::new()),
            replay_cache: Arc::new(BoundedReplayCache::new()),
            server_id: "spine-daemon-node-01".to_string(),
            allow_symmetric_auth: true,
            require_tls: false,
        }
    }
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

/// Safely resolves and canonicalizes repository paths to prevent directory traversal.
fn resolve_safe_repo_path(
    repositories_root: &Path,
    requested_path: &str,
) -> Result<PathBuf, ServerError> {
    let trimmed = requested_path.trim().trim_matches('/').replace('\\', "/");
    if trimmed.is_empty()
        || trimmed.contains("..")
        || trimmed.starts_with('/')
        || trimmed.contains(':')
    {
        return Err(ServerError::Repository(
            "Invalid repository path: directory traversal attempt".to_string(),
        ));
    }

    let candidate = repositories_root.join(&trimmed);
    let canonical_root = match repositories_root.canonicalize() {
        Ok(c) => c,
        Err(_) => repositories_root.to_path_buf(),
    };

    let target_dir = if candidate.join(".spine").exists() {
        candidate.join(".spine")
    } else if candidate.exists() && candidate.join("objects").exists() {
        candidate // Bare repo
    } else {
        return Err(ServerError::Repository(format!(
            "Repository not found: {}",
            requested_path
        )));
    };

    let canonical_target = target_dir.canonicalize().map_err(|_| {
        ServerError::Repository(format!("Repository not found: {}", requested_path))
    })?;

    if !canonical_target.starts_with(&canonical_root) {
        return Err(ServerError::Repository(
            "Security violation: path escapes repository root".to_string(),
        ));
    }

    Ok(canonical_target)
}

/// Reads a frame with timeout.
async fn read_frame_with_timeout<S: AsyncRead + Unpin>(
    stream: &mut S,
) -> Result<Frame, ServerError> {
    match tokio::time::timeout(FRAME_TIMEOUT, Frame::read_from_async(stream)).await {
        Ok(res) => res.map_err(ServerError::Protocol),
        Err(_) => Err(ServerError::Timeout(FRAME_TIMEOUT)),
    }
}

/// Convenience entrypoint for standard TCP stream connections.
pub async fn handle_server_session(
    stream: TcpStream,
    repositories_root: PathBuf,
    secret_key: [u8; 32],
) -> Result<(), ServerError> {
    let config = ServerSessionConfig {
        secret_key,
        ..Default::default()
    };
    handle_server_session_stream(stream, repositories_root, config).await
}

/// Executes a complete server session over any asynchronous stream with state machine enforcement.
pub async fn handle_server_session_stream<S: AsyncRead + AsyncWrite + Unpin + Send>(
    mut stream: S,
    repositories_root: PathBuf,
    config: ServerSessionConfig,
) -> Result<(), ServerError> {
    let mut session_state = SessionState::Connected;

    // 1. Handshake: Expect HELLO
    let hello_frame = read_frame_with_timeout(&mut stream).await?;
    session_state.validate_frame(hello_frame.header.frame_type)?;
    let _hello = HelloMessage::from_frame(&hello_frame)?;

    let ack = HelloAckMessage {
        server_version: "0.1.0".to_string(),
        selected_version: 1,
        capabilities: vec![
            "pack_zstd".to_string(),
            "ed25519_auth".to_string(),
            "cas_push".to_string(),
            "closure_validation".to_string(),
        ],
        server_id: config.server_id.clone(),
    };
    ack.to_frame()?.write_to_async(&mut stream).await?;
    session_state = SessionState::VersionNegotiated;

    // 2. Authentication challenge
    let mut nonce = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut nonce);
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    let session_id = (nonce[0] as u32) | ((nonce[1] as u32) << 8);
    let challenge = AuthChallenge {
        nonce,
        timestamp_ms: now_ms,
        session_id,
        server_id: config.server_id.clone(),
    };
    challenge.to_frame()?.write_to_async(&mut stream).await?;

    let auth_resp_frame = read_frame_with_timeout(&mut stream).await?;
    session_state.validate_frame(auth_resp_frame.header.frame_type)?;
    let auth_resp = AuthResponse::from_frame(&auth_resp_frame)?;

    // Verify authentication: prefer CapabilityTicket, then Ed25519 asymmetric, fall back to symmetric if enabled
    let mut authenticated_principal = "anonymous".to_string();
    let mut user_role = Role::Reader;
    let mut is_authed = false;
    let mut auth_key_id: Option<String> = None;
    let mut verified_ticket: Option<crate::auth::CapabilityTicket> = None;

    if auth_resp.user_id.starts_with("ticket:") {
        let ticket_str = auth_resp.user_id.trim_start_matches("ticket:");
        let now_nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as i64;
        match crate::auth::CapabilityTicket::decode_and_verify(
            ticket_str,
            &config.secret_key,
            now_nanos,
        ) {
            Ok(ticket) => {
                is_authed = true;
                authenticated_principal = ticket.username.clone();
                user_role = ticket.role;
                auth_key_id = Some(ticket.ticket_id.clone());
                verified_ticket = Some(ticket);
            }
            Err(e) => {
                eprintln!("Capability ticket verification failed: {}", e);
            }
        }
    } else if let (Some(pk), true) = (&auth_resp.public_key, auth_resp.signature.len() == 64) {
        let transcript = AuthTranscript::new(
            1,
            config.server_id.clone(),
            session_id,
            nonce,
            now_ms,
            "".to_string(),
            auth_resp
                .scope
                .clone()
                .unwrap_or_else(|| "default".to_string()),
        );

        match verify_asymmetric_auth(
            &transcript,
            pk,
            &auth_resp.signature,
            &config.key_registry,
            &config.replay_cache,
        ) {
            Ok(entry) => {
                is_authed = true;
                authenticated_principal = entry.principal;
                user_role = entry.role;
                auth_key_id = auth_resp.key_id.clone();
            }
            Err(e) => {
                eprintln!("Asymmetric auth failed: {}", e);
            }
        }
    } else if config.allow_symmetric_auth {
        // Fallback symmetric BLAKE3 keyed hash
        if let Ok(true) = verify_symmetric_auth_signature(
            &config.secret_key,
            &auth_resp.user_id,
            &nonce,
            now_ms,
            &auth_resp.signature,
            &config.replay_cache,
        ) {
            is_authed = true;
            authenticated_principal = auth_resp.user_id.clone();
            user_role = Role::Admin; // Development symmetric secret grants Admin
        } else if verify_auth_signature(
            &config.secret_key,
            &auth_resp.user_id,
            &nonce,
            now_ms,
            &auth_resp.signature,
        )
        .unwrap_or(false)
        {
            is_authed = true;
            authenticated_principal = auth_resp.user_id.clone();
            user_role = Role::Contributor;
        }
    }

    let auth_result = AuthResult {
        authenticated: is_authed,
        user: authenticated_principal.clone(),
        role: if is_authed {
            user_role.to_string()
        } else {
            "none".to_string()
        },
        message: if is_authed {
            "Authentication successful".to_string()
        } else {
            "Invalid credentials".to_string()
        },
        key_id: auth_key_id,
    };
    auth_result.to_frame()?.write_to_async(&mut stream).await?;

    if !is_authed {
        return Ok(());
    }

    session_state = SessionState::Authenticated;

    // 3. Repository selection & Anti-enumeration privacy
    let repo_open_frame = read_frame_with_timeout(&mut stream).await?;
    session_state.validate_frame(repo_open_frame.header.frame_type)?;
    let repo_open = RepoOpen::from_frame(&repo_open_frame)?;

    let spine_dir = match resolve_safe_repo_path(&repositories_root, &repo_open.path) {
        Ok(p) => p,
        Err(_) => {
            // Anti-enumeration: Return 404 regardless of whether repo doesn't exist or is unauthorized
            let err = ErrorMessage {
                code: 404,
                category: "not_found".to_string(),
                message: format!("Repository not found: {}", repo_open.path),
            };
            err.to_frame()?.write_to_async(&mut stream).await?;
            return Ok(());
        }
    };

    // If authenticated via capability ticket, verify ticket matches target repo
    if let Some(ref ticket) = verified_ticket {
        let req_clean = repo_open.path.trim().trim_matches('/').replace('\\', "/");
        let matches_repo = req_clean == ticket.native_repo_id
            || req_clean == ticket.repo_id
            || req_clean.ends_with(&ticket.native_repo_id)
            || req_clean.replace('/', "_") == ticket.native_repo_id;
        if !matches_repo {
            let err = ErrorMessage {
                code: 403,
                category: "forbidden".to_string(),
                message: "Capability ticket is not authorized for requested repository".to_string(),
            };
            err.to_frame()?.write_to_async(&mut stream).await?;
            return Ok(());
        }
    }

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

    session_state = SessionState::RepositoryOpen;

    // 5. Command loop
    loop {
        let frame = match read_frame_with_timeout(&mut stream).await {
            Ok(f) => f,
            Err(ServerError::Protocol(ProtocolError::UnexpectedEof)) => break,
            Err(e) => return Err(e),
        };

        session_state.validate_frame(frame.header.frame_type)?;

        match frame.header.frame_type {
            FrameType::FetchNegotiate => {
                if !user_role.can_read() {
                    let err = ErrorMessage {
                        code: 403,
                        category: "forbidden".to_string(),
                        message: "Read permission denied".to_string(),
                    };
                    err.to_frame()?.write_to_async(&mut stream).await?;
                    continue;
                }

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
                if !user_role.can_push() {
                    let err = ErrorMessage {
                        code: 403,
                        category: "forbidden".to_string(),
                        message: "Push permission denied for current role".to_string(),
                    };
                    err.to_frame()?.write_to_async(&mut stream).await?;
                    continue;
                }

                let _push_begin = PushBegin::from_frame(&frame)?;
                receive_and_ingest_pack(&mut stream, &spine_dir, &store).await?;
            }
            FrameType::RefUpdateReq => {
                if !user_role.can_push() {
                    let err = ErrorMessage {
                        code: 403,
                        category: "forbidden".to_string(),
                        message: "Push permission denied for current role".to_string(),
                    };
                    err.to_frame()?.write_to_async(&mut stream).await?;
                    continue;
                }

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

                    // 1. Lease check
                    if let Some(lease) = update.lease_oid {
                        if current_oid != Some(lease) {
                            overall_success = false;
                            results.push(RefUpdateStatus {
                                ref_name: update.ref_name.clone(),
                                status: "stale_ref".to_string(),
                                current_oid,
                                message: Some(format!(
                                    "Lease mismatch: expected remote tip {}, but current tip is {:?}",
                                    lease.to_hex(),
                                    current_oid.map(|o| o.to_hex())
                                )),
                            });
                            continue;
                        }
                    }

                    // 2. Expected old OID CAS check
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

                    // 3. Protected branch and admin permission check
                    let is_protected = update.ref_name == "refs/heads/main"
                        || update.ref_name == "refs/heads/master"
                        || update.ref_name.starts_with("refs/heads/release");

                    if is_protected && !user_role.can_modify_protected() {
                        overall_success = false;
                        results.push(RefUpdateStatus {
                            ref_name: update.ref_name.clone(),
                            status: "permission_denied".to_string(),
                            current_oid,
                            message: Some(format!(
                                "Modifying protected branch '{}' requires Admin role (current role: {})",
                                update.ref_name, user_role
                            )),
                        });
                        continue;
                    }

                    // 4. Force push permissions check
                    if update.force && !user_role.can_force_push() {
                        overall_success = false;
                        results.push(RefUpdateStatus {
                            ref_name: update.ref_name.clone(),
                            status: "permission_denied".to_string(),
                            current_oid,
                            message: Some("Force push requires Admin role".to_string()),
                        });
                        continue;
                    }

                    // 5. Fast-forward reachability check
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

                    // 6. Object closure validation: verify complete transitive graph before ref publication
                    let mut boundary = HashSet::new();
                    if let Some(old) = current_oid {
                        boundary.insert(old);
                    }
                    if let Err(e) = validate_push_object_closure(&store, &update.new_oid, &boundary)
                    {
                        overall_success = false;
                        results.push(RefUpdateStatus {
                            ref_name: update.ref_name.clone(),
                            status: "missing_object".to_string(),
                            current_oid,
                            message: Some(format!("Object closure validation failed: {}", e)),
                        });
                        continue;
                    }

                    // 7. Apply update atomically
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

    let _ = session_state;
    Ok(())
}
