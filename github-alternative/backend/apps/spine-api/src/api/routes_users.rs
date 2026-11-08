use super::middleware::AppState;
use crate::auth::pat::{generate_pat, hash_pat};
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct CreatePatRequest {
    pub name: String,
    pub scopes: Option<Vec<String>>,
    pub expiry_days: Option<i64>,
}

#[derive(Serialize)]
pub struct CreatePatResponse {
    pub pat: PatDto,
    pub token: String, // raw token displayed ONCE
}

pub async fn get_me(Extension(user): Extension<User>) -> impl IntoResponse {
    Json(UserDto::from(&user))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ForgeError> {
    let sessions = state.db.list_user_sessions(&user.id).await?;
    let dtos: Vec<SessionDto> = sessions
        .into_iter()
        .map(|s| SessionDto {
            id: s.id,
            ip_address: s.ip_address,
            user_agent: s.user_agent,
            created_at_nanos: s.created_at_nanos,
            expires_at_nanos: s.expires_at_nanos,
            is_active: !s.revoked,
        })
        .collect();
    Ok(Json(dtos))
}

pub async fn revoke_session(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(session_id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let sessions = state.db.list_user_sessions(&user.id).await?;
    if !sessions.iter().any(|s| s.id == session_id) {
        return Err(ForgeError::NotFound("Session not found".into()));
    }

    state.db.revoke_session(&session_id).await?;
    Ok((StatusCode::NO_CONTENT, ()))
}

pub async fn list_pats(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ForgeError> {
    let pats = state.db.list_user_pats(&user.id).await?;
    let dtos: Vec<PatDto> = pats
        .into_iter()
        .map(|p| PatDto {
            id: p.id,
            name: p.name,
            scopes: p.scopes,
            expires_at_nanos: p.expires_at_nanos,
            created_at_nanos: p.created_at_nanos,
            last_used_at_nanos: p.last_used_at_nanos,
            revoked: p.revoked,
        })
        .collect();
    Ok(Json(dtos))
}

pub async fn create_pat(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Json(payload): Json<CreatePatRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let name = payload.name.trim();
    if name.is_empty() || name.len() > 100 {
        return Err(ForgeError::BadRequest(
            "Token name must be 1-100 characters".into(),
        ));
    }

    let scopes = payload
        .scopes
        .unwrap_or_else(|| vec!["repo:read".into(), "repo:write".into()]);
    let days = payload.expiry_days.unwrap_or(30).clamp(1, 365);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let expires_at = now + (days * 24 * 3600 * 1_000_000_000);

    let raw_token = generate_pat();
    let token_hash = hash_pat(&raw_token);
    let pat_id = format!("pat_{}", uuid::Uuid::new_v4().simple());

    let pat = PersonalAccessToken {
        id: pat_id.clone(),
        user_id: user.id.clone(),
        name: name.to_string(),
        token_hash,
        scopes: scopes.clone(),
        expires_at_nanos: expires_at,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };

    let created = state.db.create_pat(pat).await?;

    let resp = CreatePatResponse {
        pat: PatDto {
            id: created.id,
            name: created.name,
            scopes: created.scopes,
            expires_at_nanos: created.expires_at_nanos,
            created_at_nanos: created.created_at_nanos,
            last_used_at_nanos: None,
            revoked: false,
        },
        token: raw_token,
    };

    Ok((StatusCode::CREATED, Json(resp)))
}

pub async fn revoke_pat(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(pat_id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    state.db.revoke_pat(&pat_id, &user.id).await?;
    Ok((StatusCode::NO_CONTENT, ()))
}

// -----------------------------------------------------------------------------
// SSH Key Management (Batch 18)
// -----------------------------------------------------------------------------

pub async fn list_ssh_keys(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
) -> Result<impl IntoResponse, ForgeError> {
    let keys = state.db.list_user_ssh_keys(&user.id).await?;
    let dtos: Vec<UserSshKeyDto> = keys.iter().map(UserSshKeyDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_ssh_key(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Json(payload): Json<CreateSshKeyRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let title = payload.title.trim();
    if title.is_empty() || title.len() > 100 {
        return Err(ForgeError::BadRequest(
            "Key title must be between 1 and 100 characters".into(),
        ));
    }

    let parsed = russh::keys::ssh_key::PublicKey::from_openssh(payload.key.trim())
        .map_err(|e| ForgeError::BadRequest(format!("Invalid OpenSSH public key: {}", e)))?;

    let fingerprint = parsed
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();
    let key_type = parsed.algorithm().to_string();
    let public_key = parsed
        .to_openssh()
        .map_err(|e| ForgeError::BadRequest(format!("Failed to format public key: {}", e)))?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let key = UserSshKey {
        id: format!("ssh_{}", uuid::Uuid::new_v4().simple()),
        user_id: user.id.clone(),
        title: title.to_string(),
        fingerprint,
        public_key,
        key_type,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };

    let created = state.db.create_user_ssh_key(key).await?;
    Ok((StatusCode::CREATED, Json(UserSshKeyDto::from(&created))))
}

pub async fn delete_ssh_key(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Path(key_id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    state.db.revoke_user_ssh_key(&key_id, &user.id).await?;
    Ok((StatusCode::NO_CONTENT, ()))
}
