use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use spine_cluster::durability::DurabilityPolicy;

#[derive(Deserialize)]
pub struct CreateRepoApiRequest {
    pub name: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
    pub durability: Option<String>,
}

#[derive(Deserialize)]
pub struct CommitsQuery {
    pub r#ref: Option<String>,
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct TreeQuery {
    pub path: Option<String>,
}

pub async fn create_repository(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(payload): Json<CreateRepoApiRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let creator = user.ok_or_else(|| {
        ForgeError::Unauthorized("Authentication required to create repository".into())
    })?;

    let durability = match payload.durability.as_deref() {
        Some("SingleCopy") | Some("SINGLE_COPY") => Some(DurabilityPolicy::SingleCopy),
        Some("TwoCopies") | Some("TWO_COPIES") => Some(DurabilityPolicy::TwoCopies),
        Some("ThreeCopies") | Some("THREE_COPIES") => Some(DurabilityPolicy::ThreeCopies),
        _ => None,
    };

    let repo = state
        .repositories
        .create_repository(
            &creator,
            &payload.name,
            payload.description,
            payload.is_private.unwrap_or(false),
            durability,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(RepositoryDto::from(&repo))))
}

pub async fn list_repositories(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repos = state
        .repositories
        .list_repositories(caller.as_ref())
        .await?;
    let dtos: Vec<RepositoryDto> = repos.iter().map(RepositoryDto::from).collect();
    Ok(Json(dtos))
}

pub async fn get_repository(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;
    Ok(Json(RepositoryDto::from(&repo)))
}

pub async fn delete_repository(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    state
        .repositories
        .delete_repository(&caller, &owner, &name)
        .await?;
    Ok((StatusCode::NO_CONTENT, ()))
}

pub async fn list_repo_branches(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let branches = state.vcs.list_branches(&repo.native_repo_id)?;
    Ok(Json(branches))
}

pub async fn list_repo_tags(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let tags = state.vcs.list_tags(&repo.native_repo_id)?;
    Ok(Json(tags))
}

pub async fn list_repo_commits(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<CommitsQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let commits = state
        .vcs
        .get_commits(&repo.native_repo_id, query.r#ref.as_deref(), limit)?;
    Ok(Json(commits))
}

pub async fn get_repo_tree(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name, ref_name)): Path<(String, String, String)>,
    Query(query): Query<TreeQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let tree = state
        .vcs
        .get_tree(&repo.native_repo_id, &ref_name, query.path.as_deref())?;
    Ok(Json(tree))
}

pub async fn get_repo_blob(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name, ref_name, path)): Path<(String, String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let blob = state.vcs.get_blob(&repo.native_repo_id, &ref_name, &path)?;
    Ok(Json(blob))
}

#[derive(Deserialize, Default)]
pub struct CapabilityTicketRequest {
    pub operation: Option<String>,
}

#[derive(Serialize)]
pub struct CapabilityTicketResponse {
    pub ticket: String,
    pub expires_at_nanos: i64,
    pub role: String,
    pub repo_id: String,
    pub native_repo_id: String,
}

pub async fn issue_capability_ticket(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
    payload: Option<Json<CapabilityTicketRequest>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let op = payload
        .and_then(|Json(p)| p.operation)
        .unwrap_or_else(|| "read".to_string());
    let op_lower = op.to_lowercase();

    let role = if op_lower == "write" || op_lower == "push" {
        let perm_role = state
            .permissions
            .check_write_access(caller.as_ref(), &repo)
            .await?;
        match perm_role {
            PermissionRole::Owner => spine_net::auth::Role::Admin,
            PermissionRole::Collaborator => spine_net::auth::Role::Contributor,
            _ => {
                return Err(ForgeError::Forbidden(
                    "Write permission required for this repository".into(),
                ))
            }
        }
    } else if op_lower == "admin" || op_lower == "owner" {
        let perm_role = state
            .permissions
            .check_write_access(caller.as_ref(), &repo)
            .await?;
        match perm_role {
            PermissionRole::Owner => spine_net::auth::Role::Admin,
            _ => {
                return Err(ForgeError::Forbidden(
                    "Admin permission required for this repository".into(),
                ))
            }
        }
    } else {
        let perm_role = state
            .permissions
            .check_read_access(caller.as_ref(), &repo)
            .await?;
        match perm_role {
            PermissionRole::Owner => spine_net::auth::Role::Admin,
            PermissionRole::Collaborator => spine_net::auth::Role::Contributor,
            PermissionRole::Reader => spine_net::auth::Role::Reader,
        }
    };

    let (user_id, username) = match caller {
        Some(u) => (u.id, u.username),
        None => ("anon_user".to_string(), "anonymous".to_string()),
    };

    let now_nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as i64;
    let validity_nanos = 3600 * 1_000_000_000i64; // 1 hour

    let ticket = spine_net::auth::CapabilityTicket::new(
        user_id,
        username,
        repo.id.clone(),
        repo.native_repo_id.clone(),
        role,
        validity_nanos,
        now_nanos,
    );

    let signed_ticket = ticket.encode_and_sign(spine_net::auth::DEFAULT_SECRET_KEY);

    Ok(Json(CapabilityTicketResponse {
        ticket: signed_ticket,
        expires_at_nanos: ticket.expires_at_nanos,
        role: role.to_string(),
        repo_id: repo.id,
        native_repo_id: repo.native_repo_id,
    }))
}

pub async fn get_repo_durability_status(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    let report = state.vcs.verify_durability(&repo.native_repo_id).await?;
    Ok(Json(report))
}

// -----------------------------------------------------------------------------
// Repository Deploy Keys (Batch 18)
// -----------------------------------------------------------------------------

pub async fn list_deploy_keys(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    state
        .permissions
        .check_write_access(caller.as_ref(), &repo)
        .await?;

    let keys = state.db.list_deploy_keys(&repo.id).await?;
    let dtos: Vec<DeployKeyDto> = keys.iter().map(DeployKeyDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_deploy_key(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name)): Path<(String, String)>,
    Json(payload): Json<CreateDeployKeyRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    state
        .permissions
        .check_admin_access(caller.as_ref(), &repo)
        .await?;

    let title = payload.title.trim();
    if title.is_empty() || title.len() > 100 {
        return Err(ForgeError::BadRequest(
            "Deploy key title must be between 1 and 100 characters".into(),
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
    let key = DeployKey {
        id: format!("deploy_key_{}", uuid::Uuid::new_v4().simple()),
        repository_id: repo.id.clone(),
        title: title.to_string(),
        fingerprint,
        public_key,
        key_type,
        write_enabled: payload.write_enabled,
        created_at_nanos: now,
        last_used_at_nanos: None,
        revoked: false,
    };

    let created = state.db.create_deploy_key(key).await?;
    Ok((StatusCode::CREATED, Json(DeployKeyDto::from(&created))))
}

pub async fn delete_deploy_key(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, name, key_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller.as_ref(), &owner, &name)
        .await?;

    state
        .permissions
        .check_admin_access(caller.as_ref(), &repo)
        .await?;
    state.db.revoke_deploy_key(&key_id, &repo.id).await?;
    Ok((StatusCode::NO_CONTENT, ()))
}
