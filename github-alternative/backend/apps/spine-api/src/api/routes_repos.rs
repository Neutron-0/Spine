use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
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
    let creator = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to create repository".into()))?;

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
    let repos = state.repositories.list_repositories(caller.as_ref()).await?;
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

    let blob = state
        .vcs
        .get_blob(&repo.native_repo_id, &ref_name, &path)?;
    Ok(Json(blob))
}
