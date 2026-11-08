use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CreateTeamRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct AddTeamMemberRequest {
    pub username: String,
    pub is_maintainer: Option<bool>,
}

#[derive(Deserialize)]
pub struct SetTeamRepoPermissionRequest {
    pub permission: String, // READ, WRITE, MAINTAIN, ADMIN
}

pub async fn list_teams(
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
    let teams = state.db.list_teams_for_org(&org.id).await?;
    let dtos: Vec<TeamDto> = teams.iter().map(TeamDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_team(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<CreateTeamRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let name = payload.name.trim().to_string();
    if name.is_empty() || name.len() > 100 {
        return Err(ForgeError::BadRequest("Invalid team name length".into()));
    }
    let slug = name.to_lowercase().replace(' ', "-");
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let team_id = format!("team_{}", uuid::Uuid::new_v4().simple());

    let team = Team {
        id: team_id.clone(),
        org_id: org.id.clone(),
        name: name.clone(),
        normalized_slug: slug,
        description: payload.description.unwrap_or_default(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_team(team).await?;

    // Add creator as maintainer
    let membership = TeamMembership {
        id: format!("tm_{}", uuid::Uuid::new_v4().simple()),
        team_id: created.id.clone(),
        user_id: caller.id.clone(),
        is_maintainer: true,
        created_at_nanos: now,
    };
    state.db.add_team_member(membership).await?;

    Ok((StatusCode::CREATED, Json(TeamDto::from(&created))))
}

pub async fn get_team(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug)): Path<(String, String)>,
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

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    Ok(Json(TeamDto::from(&team)))
}

pub async fn delete_team(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    state.db.delete_team(&team.id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_team_members(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug)): Path<(String, String)>,
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

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    let members = state.db.list_team_members(&team.id).await?;
    let mut dtos = Vec::new();
    for m in members {
        if let Some(u) = state.db.get_user_by_id(&m.user_id).await? {
            dtos.push(TeamMemberDto {
                user: UserDto::from(&u),
                is_maintainer: m.is_maintainer,
                created_at_nanos: m.created_at_nanos,
            });
        }
    }
    Ok(Json(dtos))
}

pub async fn add_team_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug)): Path<(String, String)>,
    Json(payload): Json<AddTeamMemberRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    let target_user = state
        .db
        .get_user_by_username(&payload.username)
        .await?
        .ok_or_else(|| ForgeError::NotFound("User not found".into()))?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let membership = TeamMembership {
        id: format!("tm_{}", uuid::Uuid::new_v4().simple()),
        team_id: team.id.clone(),
        user_id: target_user.id.clone(),
        is_maintainer: payload.is_maintainer.unwrap_or(false),
        created_at_nanos: now,
    };
    state.db.add_team_member(membership).await?;

    Ok(StatusCode::OK)
}

pub async fn remove_team_member(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug, user_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    state.db.remove_team_member(&team.id, &user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn set_team_repository_permission(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, team_slug, repo_name)): Path<(String, String, String)>,
    Json(payload): Json<SetTeamRepoPermissionRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let team = state
        .db
        .get_team_by_slug(&org.id, &team_slug)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Team not found".into()))?;

    let repo = state
        .db
        .get_repository_by_owner_name(&org.name, &repo_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;

    let level = TeamPermissionLevel::from_str_lossy(&payload.permission);
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let perm = TeamRepositoryPermission {
        id: format!("trp_{}", uuid::Uuid::new_v4().simple()),
        team_id: team.id.clone(),
        repo_id: repo.id.clone(),
        permission_level: level,
        created_at_nanos: now,
    };
    state.db.set_team_repository_permission(perm).await?;

    Ok(StatusCode::OK)
}
