use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::repository::service::RepositoryService;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use spine_cluster::durability::DurabilityPolicy;

#[derive(Deserialize)]
pub struct CreateOrgRequest {
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdateOrgRequest {
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub avatar_url: Option<String>,
    pub is_private: Option<bool>,
}

#[derive(Deserialize)]
pub struct AddMemberRequest {
    pub username: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateInvitationRequest {
    pub email: String,
    pub role: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateOrgRepoRequest {
    pub name: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
    pub durability: Option<String>,
}

pub async fn list_organizations(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let orgs = state.db.list_organizations_for_user(&caller.id).await?;
    let dtos: Vec<OrganizationDto> = orgs.iter().map(OrganizationDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_organization(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(payload): Json<CreateOrgRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let creator = user.ok_or_else(|| {
        ForgeError::Unauthorized("Authentication required to create organization".into())
    })?;
    let name = payload.name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err(ForgeError::BadRequest(
            "Invalid organization name length".into(),
        ));
    }
    let norm = name.to_lowercase();
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let org_id = format!("org_{}", uuid::Uuid::new_v4().simple());

    let org = Organization {
        id: org_id.clone(),
        name: name.clone(),
        normalized_name: norm,
        display_name: payload.display_name.unwrap_or_else(|| name.clone()),
        description: payload.description.unwrap_or_default(),
        avatar_url: None,
        owner_user_id: creator.id.clone(),
        is_private: payload.is_private.unwrap_or(false),
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_organization(org).await?;

    // Add creator as OWNER in organization_memberships
    let membership = OrganizationMembership {
        id: format!("mem_{}", uuid::Uuid::new_v4().simple()),
        org_id: created.id.clone(),
        user_id: creator.id.clone(),
        role: OrganizationRole::Owner,
        created_at_nanos: now,
    };
    state.db.add_organization_member(membership).await?;

    Ok((StatusCode::CREATED, Json(OrganizationDto::from(&created))))
}

pub async fn get_organization(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state
        .permissions
        .check_org_read_access(caller, &org)
        .await?;
    Ok(Json(OrganizationDto::from(&org)))
}

pub async fn update_organization(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<UpdateOrgRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let mut org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    if let Some(dn) = payload.display_name {
        org.display_name = dn;
    }
    if let Some(desc) = payload.description {
        org.description = desc;
    }
    if let Some(avatar) = payload.avatar_url {
        org.avatar_url = Some(avatar);
    }
    if let Some(priv_flag) = payload.is_private {
        org.is_private = priv_flag;
    }
    org.updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let updated = state.db.update_organization(org).await?;
    Ok(Json(OrganizationDto::from(&updated)))
}

pub async fn delete_organization(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    if org.owner_user_id != caller.id {
        return Err(ForgeError::Forbidden(
            "Only organization owner may delete organization".into(),
        ));
    }

    state.db.delete_organization(&org.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_organization_members(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state
        .permissions
        .check_org_read_access(caller, &org)
        .await?;
    let members = state.db.list_organization_members(&org.id).await?;

    let mut dtos = Vec::new();
    for m in members {
        if let Some(u) = state.db.get_user_by_id(&m.user_id).await? {
            dtos.push(OrganizationMemberDto {
                user: UserDto::from(&u),
                role: m.role,
                created_at_nanos: m.created_at_nanos,
            });
        }
    }
    Ok(Json(dtos))
}

pub async fn add_or_update_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<AddMemberRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let target_user = state
        .db
        .get_user_by_username(&payload.username)
        .await?
        .ok_or_else(|| ForgeError::NotFound("User not found".into()))?;

    let role = payload
        .role
        .as_deref()
        .map(OrganizationRole::from_str_lossy)
        .unwrap_or(OrganizationRole::Member);

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let membership = OrganizationMembership {
        id: format!("mem_{}", uuid::Uuid::new_v4().simple()),
        org_id: org.id.clone(),
        user_id: target_user.id.clone(),
        role,
        created_at_nanos: now,
    };
    state.db.add_organization_member(membership).await?;

    // Create notification for user
    let notif = UserNotification {
        id: format!("notif_{}", uuid::Uuid::new_v4().simple()),
        user_id: target_user.id.clone(),
        reason: "ORG_MEMBERSHIP".to_string(),
        resource_type: "organization".to_string(),
        resource_id: org.id.clone(),
        title: format!("Added to {}", org.name),
        message: format!("You were added to {} as {:?}", org.name, role),
        is_read: false,
        created_at_nanos: now,
    };
    let _ = state.db.create_notification(notif).await;

    Ok(StatusCode::OK)
}

pub async fn remove_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, user_id)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    if org.owner_user_id == user_id {
        return Err(ForgeError::Forbidden(
            "Cannot remove organization owner".into(),
        ));
    }

    state
        .db
        .remove_organization_member(&org.id, &user_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_invitations(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;
    let invs = state.db.list_organization_invitations(&org.id).await?;
    let dtos: Vec<OrganizationInvitationDto> =
        invs.iter().map(OrganizationInvitationDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_invitation(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<CreateInvitationRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let role = payload
        .role
        .as_deref()
        .map(OrganizationRole::from_str_lossy)
        .unwrap_or(OrganizationRole::Member);

    let raw_token = format!(
        "{}_{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let token_hash = blake3::hash(raw_token.as_bytes()).to_hex().to_string();
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let expires = now + 7 * 24 * 3600 * 1_000_000_000;

    let inv = OrganizationInvitation {
        id: format!("inv_{}", uuid::Uuid::new_v4().simple()),
        org_id: org.id.clone(),
        inviter_user_id: caller.id.clone(),
        invitee_email: payload.email.trim().to_lowercase(),
        role,
        token_hash: token_hash.clone(),
        status: "PENDING".to_string(),
        expires_at_nanos: expires,
        created_at_nanos: now,
        responded_at_nanos: None,
    };

    let created = state.db.create_organization_invitation(inv).await?;
    Ok((
        StatusCode::CREATED,
        Json(serde_json::json!({
            "invitation": OrganizationInvitationDto::from(&created),
            "invitation_token": raw_token
        })),
    ))
}

pub async fn accept_invitation(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(token): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| {
        ForgeError::Unauthorized("Authentication required to accept invitation".into())
    })?;
    let token_hash = blake3::hash(token.as_bytes()).to_hex().to_string();
    let inv = state
        .db
        .get_organization_invitation_by_token(&token_hash)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Invitation not found or expired".into()))?;

    if inv.status != "PENDING" {
        return Err(ForgeError::BadRequest(
            "Invitation is no longer pending".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    if now > inv.expires_at_nanos {
        return Err(ForgeError::BadRequest("Invitation has expired".into()));
    }

    state
        .db
        .update_organization_invitation_status(&inv.id, "ACCEPTED", now)
        .await?;

    let membership = OrganizationMembership {
        id: format!("mem_{}", uuid::Uuid::new_v4().simple()),
        org_id: inv.org_id.clone(),
        user_id: caller.id.clone(),
        role: inv.role,
        created_at_nanos: now,
    };
    state.db.add_organization_member(membership).await?;

    Ok(StatusCode::OK)
}

pub async fn list_org_repositories(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state
        .permissions
        .check_org_read_access(caller, &org)
        .await?;
    let repos = state.db.list_repositories_for_org(&org.id).await?;

    let mut dtos = Vec::new();
    for r in repos {
        if state
            .permissions
            .check_read_access(caller, &r)
            .await
            .is_ok()
        {
            dtos.push(RepositoryDto::from(&r));
        }
    }
    Ok(Json(dtos))
}

pub async fn create_org_repository(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<CreateOrgRepoRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let durability = match payload.durability.as_deref() {
        Some("SingleCopy") | Some("SINGLE_COPY") => Some(DurabilityPolicy::SingleCopy),
        Some("TwoCopies") | Some("TWO_COPIES") => Some(DurabilityPolicy::TwoCopies),
        Some("ThreeCopies") | Some("THREE_COPIES") => Some(DurabilityPolicy::ThreeCopies),
        _ => None,
    };

    // Create repository owned by the organization
    let valid_name = RepositoryService::validate_repo_name(&payload.name)?;
    let normalized = valid_name.to_lowercase();
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let repo_id = format!("repo_{}", uuid::Uuid::new_v4().simple());
    let native_repo_id = format!("{}_{}", org.name.to_lowercase(), normalized);
    let policy = durability.unwrap_or(DurabilityPolicy::ThreeCopies);

    let repo = ForgeRepository {
        id: repo_id.clone(),
        owner_user_id: caller.id.clone(),
        owner_username: org.name.clone(),
        owner_org_id: Some(org.id.clone()),
        name: valid_name,
        normalized_name: normalized,
        description: payload.description.unwrap_or_default(),
        is_private: payload.is_private.unwrap_or(false),
        default_branch: "main".to_string(),
        native_repo_id,
        durability_policy: policy,
        provisioning_status: ProvisioningStatus::Pending,
        created_at_nanos: now,
        updated_at_nanos: now,
        archived: false,
    };

    let created_repo = state.db.create_repository(repo).await?;

    // Grant creator Owner permission directly
    let perm = RepositoryPermission {
        id: format!("perm_{}", uuid::Uuid::new_v4().simple()),
        repo_id: created_repo.id.clone(),
        user_id: caller.id.clone(),
        role: PermissionRole::Owner,
        created_at_nanos: now,
    };
    let _ = state.db.create_permission(perm).await;

    // Outbox job
    let job = ProvisioningJob {
        id: format!("job_{}", uuid::Uuid::new_v4().simple()),
        repo_id: created_repo.id.clone(),
        status: ProvisioningStatus::Pending,
        attempts: 0,
        last_error: None,
        idempotency_key: format!("prov_{}", created_repo.id),
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    let _ = state.db.create_provisioning_job(job).await;

    Ok((
        StatusCode::CREATED,
        Json(RepositoryDto::from(&created_repo)),
    ))
}
