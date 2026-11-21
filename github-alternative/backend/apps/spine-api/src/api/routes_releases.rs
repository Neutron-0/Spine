use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, StatusCode},
    response::Response,
    Extension, Json,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ListReleasesQuery {
    pub include_drafts: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UploadAssetQuery {
    pub name: String,
    pub content_type: Option<String>,
}

/// POST /api/v1/repos/:owner/:repo/releases
pub async fn create_release(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
    user: Option<Extension<User>>,
    Json(payload): Json<CreateReleaseRequest>,
) -> Result<(StatusCode, Json<ReleaseDto>), ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let current_user = user
        .ok_or_else(|| {
            ForgeError::Unauthorized("Authentication required to create release".to_string())
        })?
        .0;
    let repo = state
        .repositories
        .get_repository(Some(&current_user), &owner, &repo_name)
        .await?;

    state
        .permissions
        .check_write_access(Some(&current_user), &repo)
        .await?;

    let dto = state
        .releases
        .create_release(&repo.id, &current_user.id, payload)
        .await?;

    Ok((StatusCode::CREATED, Json(dto)))
}

/// GET /api/v1/repos/:owner/:repo/releases
pub async fn list_releases(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
    user: Option<Extension<User>>,
    Query(query): Query<ListReleasesQuery>,
) -> Result<Json<Vec<ReleaseDto>>, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    state.permissions.check_read_access(caller, &repo).await?;

    let include_drafts = query.include_drafts.unwrap_or(false);
    let list = state
        .releases
        .list_releases(&repo.id, include_drafts)
        .await?;
    Ok(Json(list))
}

/// GET /api/v1/repos/:owner/:repo/releases/:release_id
pub async fn get_release(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
) -> Result<Json<ReleaseDto>, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    state.permissions.check_read_access(caller, &repo).await?;

    let rel = state.releases.get_release(&release_id).await?;
    if rel.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }
    Ok(Json(rel))
}

/// GET /api/v1/repos/:owner/:repo/releases/tags/:tag
pub async fn get_release_by_tag(
    State(state): State<AppState>,
    Path((owner, repo_name, tag)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
) -> Result<Json<ReleaseDto>, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    state.permissions.check_read_access(caller, &repo).await?;

    let rel = state.releases.get_release_by_tag(&repo.id, &tag).await?;
    Ok(Json(rel))
}

/// PATCH /api/v1/repos/:owner/:repo/releases/:release_id
pub async fn update_release(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
    Json(payload): Json<UpdateReleaseRequest>,
) -> Result<Json<ReleaseDto>, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let current_user = user
        .ok_or_else(|| {
            ForgeError::Unauthorized("Authentication required to update release".to_string())
        })?
        .0;
    let repo = state
        .repositories
        .get_repository(Some(&current_user), &owner, &repo_name)
        .await?;

    state
        .permissions
        .check_write_access(Some(&current_user), &repo)
        .await?;

    let existing = state.releases.get_release(&release_id).await?;
    if existing.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }

    let updated = state.releases.update_release(&release_id, payload).await?;
    Ok(Json(updated))
}

/// DELETE /api/v1/repos/:owner/:repo/releases/:release_id
pub async fn delete_release(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
) -> Result<StatusCode, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let current_user = user
        .ok_or_else(|| {
            ForgeError::Unauthorized("Authentication required to delete release".to_string())
        })?
        .0;
    let repo = state
        .repositories
        .get_repository(Some(&current_user), &owner, &repo_name)
        .await?;

    state
        .permissions
        .check_admin_access(Some(&current_user), &repo)
        .await?;

    let existing = state.releases.get_release(&release_id).await?;
    if existing.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }

    state.releases.delete_release(&release_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/repos/:owner/:repo/releases/:release_id/assets
pub async fn upload_release_asset(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id)): Path<(String, String, String)>,
    Query(query): Query<UploadAssetQuery>,
    user: Option<Extension<User>>,
    body: Bytes,
) -> Result<(StatusCode, Json<ReleaseAssetDto>), ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let current_user = user
        .ok_or_else(|| {
            ForgeError::Unauthorized("Authentication required to upload asset".to_string())
        })?
        .0;
    let repo = state
        .repositories
        .get_repository(Some(&current_user), &owner, &repo_name)
        .await?;

    state
        .permissions
        .check_write_access(Some(&current_user), &repo)
        .await?;

    let rel = state.releases.get_release(&release_id).await?;
    if rel.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }

    let content_type = query
        .content_type
        .unwrap_or_else(|| "application/octet-stream".to_string());

    let asset = state
        .releases
        .upload_asset(&release_id, &query.name, &content_type, &body)
        .await?;

    Ok((StatusCode::CREATED, Json(asset)))
}

/// GET /api/v1/repos/:owner/:repo/releases/:release_id/assets/:asset_id/download
pub async fn download_release_asset(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id, asset_id)): Path<(String, String, String, String)>,
    user: Option<Extension<User>>,
) -> Result<Response, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    state.permissions.check_read_access(caller, &repo).await?;

    let rel = state.releases.get_release(&release_id).await?;
    if rel.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }

    let (asset, bytes) = state.releases.download_asset(&asset_id).await?;
    if asset.release_id != release_id {
        return Err(ForgeError::NotFound("Asset not found in release".into()));
    }

    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, asset.content_type)
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", asset.name),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    Ok(res)
}

/// DELETE /api/v1/repos/:owner/:repo/releases/:release_id/assets/:asset_id
pub async fn delete_release_asset(
    State(state): State<AppState>,
    Path((owner, repo_name, release_id, asset_id)): Path<(String, String, String, String)>,
    user: Option<Extension<User>>,
) -> Result<StatusCode, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let current_user = user
        .ok_or_else(|| {
            ForgeError::Unauthorized("Authentication required to delete asset".to_string())
        })?
        .0;
    let repo = state
        .repositories
        .get_repository(Some(&current_user), &owner, &repo_name)
        .await?;

    state
        .permissions
        .check_write_access(Some(&current_user), &repo)
        .await?;

    let rel = state.releases.get_release(&release_id).await?;
    if rel.repo_id != repo.id {
        return Err(ForgeError::NotFound(
            "Release not found in repository".into(),
        ));
    }

    let (asset, _) = state.releases.download_asset(&asset_id).await?;
    if asset.release_id != release_id {
        return Err(ForgeError::NotFound("Asset not found in release".into()));
    }

    state.releases.delete_asset(&asset_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
