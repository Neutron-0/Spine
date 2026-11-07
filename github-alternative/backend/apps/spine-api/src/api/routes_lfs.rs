use crate::api::middleware::AppState;
use crate::api::routes_git_http::{authenticate_git_user, unauthorized_response};
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use spine_artifact::{ArtifactManifest, StreamChunker, DEFAULT_CHUNK_SIZE};
use std::collections::HashMap;
use std::io::Cursor;
use subtle::ConstantTimeEq;

type HmacSha256 = Hmac<Sha256>;
pub const LFS_SECRET_SALT: &[u8] = b"spine-lfs-transfer-token-salt-v1";

pub const LFS_MEDIA_TYPE: &str = "application/vnd.git-lfs+json";

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsBatchRequest {
    pub operation: String, // "upload" | "download"
    #[serde(default)]
    pub transfers: Vec<String>,
    #[serde(default, rename = "ref")]
    pub ref_info: Option<LfsRefInfo>,
    pub objects: Vec<LfsObjectRequest>,
    #[serde(default)]
    pub hash_algo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsRefInfo {
    pub name: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectRequest {
    pub oid: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsBatchResponse {
    pub transfer: String,
    pub objects: Vec<LfsObjectResponse>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hash_algo: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectResponse {
    pub oid: String,
    pub size: u64,
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<HashMap<String, LfsAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LfsObjectError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsAction {
    pub href: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub header: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectError {
    pub code: u16,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LfsScopedToken {
    pub repo_id: String,
    pub oid: String,
    pub op: String, // "upload" | "download" | "verify"
    pub max_size: u64,
    pub exp: i64,
}

#[derive(Debug, Deserialize)]
pub struct LfsVerifyRequest {
    pub oid: String,
    pub size: u64,
}

/// Generates a signed, short-lived HMAC transfer token.
pub fn generate_scoped_token(
    repo_id: &str,
    oid: &str,
    op: &str,
    max_size: u64,
    ttl_secs: i64,
) -> (String, i64) {
    let now = chrono::Utc::now().timestamp();
    let exp = now + ttl_secs;
    let token = LfsScopedToken {
        repo_id: repo_id.to_string(),
        oid: oid.to_string(),
        op: op.to_string(),
        max_size,
        exp,
    };
    let json_bytes = serde_json::to_vec(&token).unwrap_or_default();
    let payload_hex = hex::encode(&json_bytes);

    let mut mac = HmacSha256::new_from_slice(LFS_SECRET_SALT).expect("HMAC slice valid");
    mac.update(payload_hex.as_bytes());
    let sig_hex = hex::encode(mac.finalize().into_bytes());

    (format!("{}.{}", payload_hex, sig_hex), exp)
}

/// Verifies a signed short-lived transfer token in constant time.
pub fn verify_scoped_token(
    token_str: &str,
    expected_repo_id: &str,
    expected_oid: &str,
    expected_op: &str,
) -> Result<u64, StatusCode> {
    let (payload_hex, sig_hex) = token_str.split_once('.').ok_or(StatusCode::UNAUTHORIZED)?;
    let expected_sig = {
        let mut mac = HmacSha256::new_from_slice(LFS_SECRET_SALT).expect("HMAC slice valid");
        mac.update(payload_hex.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    };

    if expected_sig
        .as_bytes()
        .ct_eq(sig_hex.as_bytes())
        .unwrap_u8()
        != 1
    {
        return Err(StatusCode::FORBIDDEN);
    }

    let json_bytes = hex::decode(payload_hex).map_err(|_| StatusCode::UNAUTHORIZED)?;
    let token: LfsScopedToken =
        serde_json::from_slice(&json_bytes).map_err(|_| StatusCode::UNAUTHORIZED)?;

    let now = chrono::Utc::now().timestamp();
    if token.exp < now {
        return Err(StatusCode::FORBIDDEN);
    }

    if token.repo_id != expected_repo_id || token.oid != expected_oid || token.op != expected_op {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(token.max_size)
}

/// Generates a signed, short-lived RemoteAuth token for SSH LFS authentication.
pub fn generate_remote_auth_token(
    secret: &[u8],
    user_id: &str,
    repo_id: &str,
    op: &str,
    exp: i64,
) -> String {
    let payload = LfsRemoteAuthToken {
        user_id: user_id.to_string(),
        repo_id: repo_id.to_string(),
        op: op.to_string(),
        exp,
    };
    let json_bytes = serde_json::to_vec(&payload).unwrap_or_default();
    let payload_hex = hex::encode(&json_bytes);

    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC slice valid");
    mac.update(b"spine-lfs-remote-auth-v1");
    mac.update(payload_hex.as_bytes());
    let sig_hex = hex::encode(mac.finalize().into_bytes());

    format!("{}.{}", payload_hex, sig_hex)
}

/// Verifies a RemoteAuth token in constant time.
pub fn verify_remote_auth_token(secret: &[u8], token_str: &str) -> Option<LfsRemoteAuthToken> {
    let (payload_hex, sig_hex) = token_str.split_once('.')?;
    let expected_sig = {
        let mut mac = HmacSha256::new_from_slice(secret).ok()?;
        mac.update(b"spine-lfs-remote-auth-v1");
        mac.update(payload_hex.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    };

    if expected_sig
        .as_bytes()
        .ct_eq(sig_hex.as_bytes())
        .unwrap_u8()
        != 1
    {
        return None;
    }

    let json_bytes = hex::decode(payload_hex).ok()?;
    let token: LfsRemoteAuthToken = serde_json::from_slice(&json_bytes).ok()?;

    let now = chrono::Utc::now().timestamp();
    if token.exp < now {
        return None;
    }

    Some(token)
}

/// Helper to enforce HTTPS outside local loopback environments.
#[allow(clippy::result_large_err)]
pub fn check_transport_security(state: &AppState, headers: &HeaderMap) -> Result<(), Response> {
    let require_https = state.public_origin.starts_with("https://")
        || std::env::var("SPINE_REQUIRE_HTTPS").as_deref() == Ok("1");

    if require_https {
        let is_forwarded_https = headers
            .get("X-Forwarded-Proto")
            .and_then(|v| v.to_str().ok())
            .map(|p| p.eq_ignore_ascii_case("https"))
            .unwrap_or(false);

        let is_loopback = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .map(|h| {
                h.starts_with("127.0.0.1") || h.starts_with("localhost") || h.starts_with("[::1]")
            })
            .unwrap_or(false);

        if !is_forwarded_https && !is_loopback {
            return Err((
                StatusCode::FORBIDDEN,
                "HTTPS is required for authenticated remote transport; plaintext Basic authentication is prohibited\n",
            )
                .into_response());
        }
    }
    Ok(())
}

/// POST `/:owner/:repo_git/info/lfs/objects/batch`
///
/// Standard Git LFS Batch API implementing object download/upload action negotiation.
pub async fn git_lfs_batch(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let payload: LfsBatchRequest = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Invalid Batch API JSON: {}\"}}\n", e),
            )
                .into_response();
        }
    };

    // 1. Adapter validation: only 'basic' transfer adapter is supported
    if !payload.transfers.is_empty() && !payload.transfers.iter().any(|t| t == "basic") {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            serde_json::to_string(&serde_json::json!({
                "message": "Unsupported transfer adapter",
                "documentation_url": "https://github.com/git-lfs/git-lfs/blob/main/docs/api/basic-transfers.md"
            }))
            .unwrap_or_default(),
        )
            .into_response();
    }

    // 2. Hash algorithm validation: only 'sha256' is supported
    if let Some(ref algo) = payload.hash_algo {
        if algo != "sha256" {
            return (
                StatusCode::UNPROCESSABLE_ENTITY,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                serde_json::to_string(&serde_json::json!({
                    "message": "Unsupported hash algorithm; expected sha256",
                    "documentation_url": "https://github.com/git-lfs/git-lfs/blob/main/docs/api/batch.md"
                }))
                .unwrap_or_default(),
            )
                .into_response();
        }
    }

    // 3. Batch limits validation
    if payload.objects.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            "{\"message\":\"Batch request contains no objects\"}\n",
        )
            .into_response();
    }

    if payload.objects.len() > 500 {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            "{\"message\":\"Batch request exceeds maximum limit of 500 objects\"}\n",
        )
            .into_response();
    }

    for obj in &payload.objects {
        if obj.oid.len() != 64 || !obj.oid.chars().all(|c| c.is_ascii_hexdigit()) {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Invalid SHA-256 OID: {}\"}}\n", obj.oid),
            )
                .into_response();
        }
    }

    // 4. Authorization check
    let caller = authenticate_git_user(&state, &headers).await;
    if payload.operation == "upload" {
        let user = match caller {
            Some(ref u) => u,
            None => return unauthorized_response(),
        };
        if let Err(e) = state
            .permissions
            .check_write_access(Some(user), &repo)
            .await
        {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
            )
                .into_response();
        }
    } else {
        if state
            .permissions
            .check_read_access(caller.as_ref(), &repo)
            .await
            .is_err()
        {
            if caller.is_none() {
                return unauthorized_response();
            }
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Repository not found\"}\n",
            )
                .into_response();
        }
    }

    let mut response_objects = Vec::with_capacity(payload.objects.len());

    if payload.operation == "upload" {
        for obj in payload.objects {
            // Check if object already exists with matching size
            if let Ok(Some(existing)) = state.db.get_artifact_manifest(&obj.oid).await {
                if existing.total_size as u64 == obj.size {
                    // Object is already fully present in storage; omit actions so git-lfs skips upload
                    response_objects.push(LfsObjectResponse {
                        oid: obj.oid,
                        size: obj.size,
                        authenticated: true,
                        actions: None,
                        error: None,
                    });
                    continue;
                }
            }

            // Issue short-lived scoped upload and verify actions
            let (upload_token, _exp) =
                generate_scoped_token(&repo.id, &obj.oid, "upload", obj.size + 4096, 3600);
            let (verify_token, _exp) =
                generate_scoped_token(&repo.id, &obj.oid, "verify", obj.size + 4096, 3600);

            let mut upload_headers = HashMap::new();
            upload_headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", upload_token),
            );
            let mut verify_headers = HashMap::new();
            verify_headers.insert(
                "Authorization".to_string(),
                format!("Bearer {}", verify_token),
            );

            let upload_action = LfsAction {
                href: format!(
                    "{}/{}/{}.git/info/lfs/objects/{}",
                    state.public_origin, owner, repo_name, obj.oid
                ),
                header: upload_headers,
                expires_at: None,
                expires_in: Some(3600),
            };

            let verify_action = LfsAction {
                href: format!(
                    "{}/{}/{}.git/info/lfs/verify",
                    state.public_origin, owner, repo_name
                ),
                header: verify_headers,
                expires_at: None,
                expires_in: Some(3600),
            };

            let mut actions = HashMap::new();
            actions.insert("upload".to_string(), upload_action);
            actions.insert("verify".to_string(), verify_action);

            response_objects.push(LfsObjectResponse {
                oid: obj.oid,
                size: obj.size,
                authenticated: true,
                actions: Some(actions),
                error: None,
            });
        }
    } else {
        // Download operation
        for obj in payload.objects {
            if let Ok(Some(existing)) = state.db.get_artifact_manifest(&obj.oid).await {
                let (download_token, _exp) = generate_scoped_token(
                    &repo.id,
                    &obj.oid,
                    "download",
                    existing.total_size as u64,
                    3600,
                );

                let mut download_headers = HashMap::new();
                download_headers.insert(
                    "Authorization".to_string(),
                    format!("Bearer {}", download_token),
                );

                let download_action = LfsAction {
                    href: format!(
                        "{}/{}/{}.git/info/lfs/objects/{}",
                        state.public_origin, owner, repo_name, obj.oid
                    ),
                    header: download_headers,
                    expires_at: None,
                    expires_in: Some(3600),
                };

                let mut actions = HashMap::new();
                actions.insert("download".to_string(), download_action);

                response_objects.push(LfsObjectResponse {
                    oid: obj.oid,
                    size: obj.size,
                    authenticated: true,
                    actions: Some(actions),
                    error: None,
                });
            } else {
                response_objects.push(LfsObjectResponse {
                    oid: obj.oid,
                    size: obj.size,
                    authenticated: true,
                    actions: None,
                    error: Some(LfsObjectError {
                        code: 404,
                        message: "Object does not exist".to_string(),
                    }),
                });
            }
        }
    }

    let resp = LfsBatchResponse {
        transfer: "basic".to_string(),
        objects: response_objects,
        hash_algo: Some("sha256".to_string()),
    };

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
        serde_json::to_string(&resp).unwrap_or_default(),
    )
        .into_response()
}

/// PUT `/:owner/:repo_git/info/lfs/objects/:oid`
///
/// Git LFS Basic Transfer upload endpoint. Streams raw binary payload, validates size and SHA-256,
/// stores chunked blocks in `LocalArtifactStore`, persists manifest in database, and updates quotas.
pub async fn git_lfs_upload_object(
    State(state): State<AppState>,
    Path((owner, repo_git, oid)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let _write_permit = match state.consistency_barrier.acquire_write_permit().await {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, "5")],
                "Maintenance in progress. Backups active, object uploads temporarily drained.\n",
            )
                .into_response();
        }
    };

    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    // 1. Authorization: check scoped transfer token or Basic auth write access
    let authorized = if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth_val.strip_prefix("Bearer ") {
            verify_scoped_token(tok.trim(), &repo.id, &oid, "upload").is_ok()
        } else {
            false
        }
    } else {
        false
    };

    if !authorized {
        let caller = authenticate_git_user(&state, &headers).await;
        if let Err(e) = state
            .permissions
            .check_write_access(caller.as_ref(), &repo)
            .await
        {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
            )
                .into_response();
        }
    }

    // 2. Validate OID format
    if oid.len() != 64 || !oid.chars().all(|c| c.is_ascii_hexdigit()) {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            "{\"message\":\"Invalid SHA-256 OID\"}\n",
        )
            .into_response();
    }

    // 3. Check and verify SHA-256 checksum across received payload
    let mut hasher = Sha256::new();
    hasher.update(&body);
    let actual_sha256 = hex::encode(hasher.finalize());

    if actual_sha256 != oid {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            serde_json::to_string(&serde_json::json!({
                "message": format!("Payload SHA-256 mismatch: expected {}, got {}", oid, actual_sha256)
            }))
            .unwrap_or_default(),
        )
            .into_response();
    }

    // 4. Ingest payload into native spine-artifact store
    let chunker = StreamChunker::new(Some(DEFAULT_CHUNK_SIZE));
    let chunk_res = match chunker.chunk_reader(
        Cursor::new(&body),
        &oid,
        Some("application/octet-stream".to_string()),
    ) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Artifact chunking failed: {}\"}}\n", e),
            )
                .into_response();
        }
    };

    for chunk in &chunk_res.chunks {
        if let Err(e) = state
            .artifacts
            .store()
            .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
        {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Failed to store chunk: {}\"}}\n", e),
            )
                .into_response();
        }
    }

    if let Err(e) = state.artifacts.store().store_manifest(&chunk_res.manifest) {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Failed to store manifest: {}\"}}\n", e),
        )
            .into_response();
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let stored = StoredArtifactManifest {
        id: oid.clone(),
        artifact_id: oid.clone(),
        whole_object_hash: oid.clone(),
        total_size: body.len() as i64,
        chunk_size: DEFAULT_CHUNK_SIZE as i32,
        chunk_count: chunk_res.manifest.chunks.len() as i32,
        content_type: Some("application/octet-stream".to_string()),
        manifest_json: serde_json::to_string(&chunk_res.manifest).unwrap_or_default(),
        created_at_nanos: now,
    };

    let _ = state.db.save_artifact_manifest(stored).await;
    let owner_id = repo.owner_org_id.as_deref().unwrap_or(&repo.owner_user_id);
    let _ = state
        .db
        .update_quota_usage(owner_id, body.len() as i64, body.len() as i64)
        .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
        "{}",
    )
        .into_response()
}

/// POST `/:owner/:repo_git/info/lfs/verify`
///
/// Git LFS Basic Transfer verification endpoint confirming persisted object existence and size.
pub async fn git_lfs_verify_object(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let verify_req: LfsVerifyRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Invalid verify request JSON\"}\n",
            )
                .into_response();
        }
    };

    // Authenticate: scoped verify token or user Basic auth
    let authorized = if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth_val.strip_prefix("Bearer ") {
            verify_scoped_token(tok.trim(), &repo.id, &verify_req.oid, "verify").is_ok()
        } else {
            false
        }
    } else {
        false
    };

    if !authorized {
        let caller = authenticate_git_user(&state, &headers).await;
        if let Err(e) = state
            .permissions
            .check_write_access(caller.as_ref(), &repo)
            .await
        {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
            )
                .into_response();
        }
    }

    match state.db.get_artifact_manifest(&verify_req.oid).await {
        Ok(Some(manifest)) => {
            if manifest.total_size as u64 != verify_req.size {
                return (
                    StatusCode::BAD_REQUEST,
                    [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                    serde_json::to_string(&serde_json::json!({
                        "message": format!(
                            "Size mismatch: expected {}, recorded {}",
                            verify_req.size, manifest.total_size
                        )
                    }))
                    .unwrap_or_default(),
                )
                    .into_response();
            }
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{}",
            )
                .into_response()
        }
        _ => (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            "{\"message\":\"Object not found\"}\n",
        )
            .into_response(),
    }
}

/// GET `/:owner/:repo_git/info/lfs/objects/:oid`
///
/// Git LFS Basic Transfer download endpoint. Streams the raw binary payload reconstructed from native artifact chunks.
pub async fn git_lfs_download_object(
    State(state): State<AppState>,
    Path((owner, repo_git, oid)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    // Authenticate: scoped download token or user Basic auth
    let authorized = if let Some(auth_val) = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(tok) = auth_val.strip_prefix("Bearer ") {
            verify_scoped_token(tok.trim(), &repo.id, &oid, "download").is_ok()
        } else {
            false
        }
    } else {
        false
    };

    if !authorized {
        let caller = authenticate_git_user(&state, &headers).await;
        if state
            .permissions
            .check_read_access(caller.as_ref(), &repo)
            .await
            .is_err()
        {
            if caller.is_none() && repo.is_private {
                return unauthorized_response();
            }
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Permission denied\"}\n",
            )
                .into_response();
        }
    }

    let stored = match state.db.get_artifact_manifest(&oid).await {
        Ok(Some(s)) => s,
        _ => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Object not found\"}\n",
            )
                .into_response()
        }
    };

    let manifest = match ArtifactManifest::from_json(&stored.manifest_json) {
        Ok(m) => m,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Failed to parse artifact manifest: {}\n", e),
            )
                .into_response()
        }
    };

    let mut payload = Vec::with_capacity(manifest.total_size as usize);
    for entry in &manifest.chunks {
        match state.artifacts.store().get_chunk(&entry.chunk_hash) {
            Ok(Some(chunk_bytes)) => payload.extend_from_slice(&chunk_bytes),
            _ => {
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!(
                        "Missing chunk {} during object hydration\n",
                        entry.chunk_hash
                    ),
                )
                    .into_response();
            }
        }
    }

    let mut resp_headers = HeaderMap::new();
    resp_headers.insert(
        header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    resp_headers.insert(
        header::CONTENT_LENGTH,
        payload.len().to_string().parse().unwrap(),
    );

    (StatusCode::OK, resp_headers, payload).into_response()
}

// ------------------------------------------------------------------------------------------------
// Backwards-Compatible REST endpoints for Forge UI & internal test fixtures
// ------------------------------------------------------------------------------------------------

pub async fn lfs_batch(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
    user: Option<Extension<User>>,
    Json(payload): Json<LfsBatchRequest>,
) -> Result<Json<LfsBatchResponse>, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    if payload.operation == "upload" {
        state.permissions.check_write_access(caller, &repo).await?;
    } else {
        state.permissions.check_read_access(caller, &repo).await?;
    }

    let mut response_objects = Vec::new();
    for obj in payload.objects {
        if let Ok(Some(_)) = state.db.get_artifact_manifest(&obj.oid).await {
            response_objects.push(LfsObjectResponse {
                oid: obj.oid,
                size: obj.size,
                authenticated: true,
                actions: None,
                error: None,
            });
        } else if payload.operation == "upload" {
            let (tok, _) =
                generate_scoped_token(&repo.id, &obj.oid, "upload", obj.size + 4096, 3600);
            let mut actions = HashMap::new();
            actions.insert(
                "upload".to_string(),
                LfsAction {
                    href: format!(
                        "{}/{}/{}.git/info/lfs/objects/{}",
                        state.public_origin, owner, repo_name, obj.oid
                    ),
                    header: HashMap::from([(
                        "Authorization".to_string(),
                        format!("Bearer {}", tok),
                    )]),
                    expires_at: None,
                    expires_in: Some(3600),
                },
            );
            response_objects.push(LfsObjectResponse {
                oid: obj.oid,
                size: obj.size,
                authenticated: true,
                actions: Some(actions),
                error: None,
            });
        } else {
            response_objects.push(LfsObjectResponse {
                oid: obj.oid,
                size: obj.size,
                authenticated: true,
                actions: None,
                error: Some(LfsObjectError {
                    code: 404,
                    message: "Object not found".to_string(),
                }),
            });
        }
    }

    Ok(Json(LfsBatchResponse {
        transfer: "basic".to_string(),
        objects: response_objects,
        hash_algo: Some("sha256".to_string()),
    }))
}

pub async fn lfs_upload_chunk(
    State(state): State<AppState>,
    Path((_owner, _repo, session_id, index, hash)): Path<(String, String, String, u32, String)>,
    body: Bytes,
) -> Result<StatusCode, ForgeError> {
    state
        .artifacts
        .upload_chunk(&session_id, index, &hash, &body)
        .await?;
    Ok(StatusCode::OK)
}

pub async fn lfs_finalize(
    State(state): State<AppState>,
    Path((_owner, _repo, session_id)): Path<(String, String, String)>,
    Json(manifest): Json<ArtifactManifest>,
) -> Result<Json<StoredArtifactManifest>, ForgeError> {
    let stored = state
        .artifacts
        .finalize_upload(&session_id, manifest)
        .await?;
    Ok(Json(stored))
}

pub async fn lfs_download_object(
    State(state): State<AppState>,
    Path((_owner, _repo, manifest_id)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let bytes = state.artifacts.reconstruct_object(&manifest_id).await?;
    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    Ok(res)
}

pub async fn lfs_download_chunk(
    State(state): State<AppState>,
    Path((_owner, _repo, chunk_hash)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let bytes = state.artifacts.get_chunk(&chunk_hash).await?;
    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    Ok(res)
}

// -----------------------------------------------------------------------------
// Git LFS File Locking Endpoints (Batch 18)
// -----------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
pub struct LfsListLocksQuery {
    pub path: Option<String>,
    pub id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<usize>,
}

/// POST `/:owner/:repo_git/info/lfs/locks`
pub async fn git_lfs_create_lock(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    Json(payload): Json<LfsCreateLockRequest>,
) -> Response {
    let _write_permit = match state.consistency_barrier.acquire_write_permit().await {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, "5")],
                "Maintenance in progress. Backups active, lock mutations temporarily drained.\n",
            )
                .into_response();
        }
    };

    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let user = match authenticate_git_user(&state, &headers).await {
        Some(u) => u,
        None => return unauthorized_response(),
    };

    if let Err(e) = state
        .permissions
        .check_write_access(Some(&user), &repo)
        .await
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
        )
            .into_response();
    }

    let normalized_path = payload.path.trim().trim_start_matches('/').to_string();
    if normalized_path.is_empty() || normalized_path.contains("..") {
        return (
            StatusCode::BAD_REQUEST,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            "{\"message\":\"Invalid path for locking\"}\n",
        )
            .into_response();
    }

    if let Ok(Some(existing)) = state
        .db
        .get_lfs_lock_by_path(&repo.id, &normalized_path)
        .await
    {
        return (
            StatusCode::CONFLICT,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            serde_json::to_string(&LfsCreateLockResponse {
                lock: LfsLockDto::from(&existing),
                message: Some("already locked".to_string()),
            })
            .unwrap_or_default(),
        )
            .into_response();
    }

    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let new_lock = LfsLock {
        id: uuid::Uuid::new_v4().to_string(),
        repository_id: repo.id.clone(),
        path: normalized_path.clone(),
        owner_id: user.id.clone(),
        owner_username: user.username.clone(),
        locked_at_nanos: now_nanos,
    };

    let created = match state.db.create_lfs_lock(new_lock).await {
        Ok(l) => l,
        Err(ForgeError::Conflict(_)) => {
            if let Ok(Some(existing)) = state
                .db
                .get_lfs_lock_by_path(&repo.id, &normalized_path)
                .await
            {
                return (
                    StatusCode::CONFLICT,
                    [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                    serde_json::to_string(&LfsCreateLockResponse {
                        lock: LfsLockDto::from(&existing),
                        message: Some("already locked".to_string()),
                    })
                    .unwrap_or_default(),
                )
                    .into_response();
            }
            return (
                StatusCode::CONFLICT,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Conflict locking file\"}\n",
            )
                .into_response();
        }
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!("{{\"message\":\"Failed to create lock: {}\"}}\n", e),
            )
                .into_response();
        }
    };

    let _ = state
        .db
        .create_lfs_lock_audit(LfsLockAudit {
            id: uuid::Uuid::new_v4().to_string(),
            repository_id: repo.id.clone(),
            path: normalized_path,
            actor_user_id: user.id,
            action: "LOCK".to_string(),
            created_at_nanos: now_nanos,
        })
        .await;

    (
        StatusCode::CREATED,
        [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
        serde_json::to_string(&LfsCreateLockResponse {
            lock: LfsLockDto::from(&created),
            message: None,
        })
        .unwrap_or_default(),
    )
        .into_response()
}

/// GET `/:owner/:repo_git/info/lfs/locks`
pub async fn git_lfs_list_locks(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    Query(query): Query<LfsListLocksQuery>,
    headers: HeaderMap,
) -> Response {
    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let caller = authenticate_git_user(&state, &headers).await;
    if let Err(e) = state
        .permissions
        .check_read_access(caller.as_ref(), &repo)
        .await
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Read permission denied: {}\"}}\n", e),
        )
            .into_response();
    }

    if let Some(ref lock_id) = query.id {
        if let Ok(Some(lock)) = state.db.get_lfs_lock_by_id(lock_id).await {
            if lock.repository_id == repo.id {
                let resp = LfsListLocksResponse {
                    locks: vec![LfsLockDto::from(&lock)],
                    next_cursor: None,
                };
                return (
                    StatusCode::OK,
                    [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                    serde_json::to_string(&resp).unwrap_or_default(),
                )
                    .into_response();
            }
        }
        let resp = LfsListLocksResponse {
            locks: vec![],
            next_cursor: None,
        };
        return (
            StatusCode::OK,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            serde_json::to_string(&resp).unwrap_or_default(),
        )
            .into_response();
    }

    let norm_path = query
        .path
        .as_deref()
        .map(|p| p.trim().trim_start_matches('/'));
    match state
        .db
        .list_lfs_locks(&repo.id, norm_path, query.cursor.as_deref(), query.limit)
        .await
    {
        Ok((locks, next_cursor)) => {
            let dtos: Vec<LfsLockDto> = locks.iter().map(LfsLockDto::from).collect();
            let resp = LfsListLocksResponse {
                locks: dtos,
                next_cursor,
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                serde_json::to_string(&resp).unwrap_or_default(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Failed to list locks: {}\"}}\n", e),
        )
            .into_response(),
    }
}

/// POST `/:owner/:repo_git/info/lfs/locks/verify`
pub async fn git_lfs_verify_locks(
    State(state): State<AppState>,
    Path((owner, repo_git)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let user = match authenticate_git_user(&state, &headers).await {
        Some(u) => u,
        None => return unauthorized_response(),
    };

    if let Err(e) = state
        .permissions
        .check_write_access(Some(&user), &repo)
        .await
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
        )
            .into_response();
    }

    let req: LfsVerifyLocksRequest = serde_json::from_slice(&body).unwrap_or_default();
    match state
        .db
        .list_lfs_locks(&repo.id, None, req.cursor.as_deref(), req.limit)
        .await
    {
        Ok((locks, next_cursor)) => {
            let mut ours = Vec::new();
            let mut theirs = Vec::new();
            for lock in locks {
                let dto = LfsLockDto::from(&lock);
                if lock.owner_id == user.id {
                    ours.push(dto);
                } else {
                    theirs.push(dto);
                }
            }
            let resp = LfsVerifyLocksResponse {
                ours,
                theirs,
                next_cursor,
            };
            (
                StatusCode::OK,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                serde_json::to_string(&resp).unwrap_or_default(),
            )
                .into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Failed to verify locks: {}\"}}\n", e),
        )
            .into_response(),
    }
}

/// POST `/:owner/:repo_git/info/lfs/locks/:id/unlock`
pub async fn git_lfs_unlock(
    State(state): State<AppState>,
    Path((owner, repo_git, lock_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let _write_permit = match state.consistency_barrier.acquire_write_permit().await {
        Ok(permit) => permit,
        Err(_) => {
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                [(axum::http::header::RETRY_AFTER, "5")],
                "Maintenance in progress. Backups active, lock mutations temporarily drained.\n",
            )
                .into_response();
        }
    };

    if let Err(resp) = check_transport_security(&state, &headers) {
        return resp;
    }

    let repo_name = repo_git.strip_suffix(".git").unwrap_or(&repo_git);
    let repo = match state
        .db
        .get_repository_by_owner_name(&owner, repo_name)
        .await
    {
        Ok(Some(r)) => r,
        _ => return (StatusCode::NOT_FOUND, "Repository not found\n").into_response(),
    };

    let user = match authenticate_git_user(&state, &headers).await {
        Some(u) => u,
        None => return unauthorized_response(),
    };

    if let Err(e) = state
        .permissions
        .check_write_access(Some(&user), &repo)
        .await
    {
        return (
            StatusCode::FORBIDDEN,
            [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
            format!("{{\"message\":\"Write permission denied: {}\"}}\n", e),
        )
            .into_response();
    }

    let req: LfsUnlockRequest = serde_json::from_slice(&body).unwrap_or_default();
    let force = req.force.unwrap_or(false);

    let lock = match state.db.get_lfs_lock_by_id(&lock_id).await {
        Ok(Some(l)) => {
            if l.repository_id != repo.id {
                return (
                    StatusCode::NOT_FOUND,
                    [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                    "{\"message\":\"Lock not found in repository\"}\n",
                )
                    .into_response();
            }
            l
        }
        _ => {
            return (
                StatusCode::NOT_FOUND,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Lock not found\"}\n",
            )
                .into_response();
        }
    };

    let action_str;
    if lock.owner_id != user.id {
        if !force {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                format!(
                    "{{\"message\":\"Lock belongs to '{}'; cannot unlock without force\"}}\n",
                    lock.owner_username
                ),
            )
                .into_response();
        }
        if state
            .permissions
            .check_admin_access(Some(&user), &repo)
            .await
            .is_err()
        {
            return (
                StatusCode::FORBIDDEN,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Only repository administrators may force unlock files\"}\n",
            )
                .into_response();
        }
        action_str = "FORCE_UNLOCK";
    } else {
        action_str = "UNLOCK";
    }

    let deleted = match state.db.delete_lfs_lock(&lock_id).await {
        Ok(Some(l)) => l,
        _ => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
                "{\"message\":\"Failed to delete lock\"}\n",
            )
                .into_response();
        }
    };

    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let _ = state
        .db
        .create_lfs_lock_audit(LfsLockAudit {
            id: uuid::Uuid::new_v4().to_string(),
            repository_id: repo.id.clone(),
            path: deleted.path.clone(),
            actor_user_id: user.id,
            action: action_str.to_string(),
            created_at_nanos: now_nanos,
        })
        .await;

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, LFS_MEDIA_TYPE)],
        serde_json::to_string(&LfsUnlockResponse {
            lock: LfsLockDto::from(&deleted),
            message: None,
        })
        .unwrap_or_default(),
    )
        .into_response()
}
