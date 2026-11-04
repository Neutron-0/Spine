use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::User;

#[derive(Debug, Deserialize)]
pub struct UpdateProfileRequest {
    pub bio: Option<String>,
    pub company: Option<String>,
    pub location: Option<String>,
    pub website_url: Option<String>,
    pub twitter_username: Option<String>,
    pub profile_readme: Option<String>,
    pub pinned_repo_ids: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
pub struct SetTopicsRequest {
    pub topics: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct ExploreQuery {
    pub topic: Option<String>,
}

async fn resolve_repo(state: &AppState, owner: &str, repo_name: &str) -> Result<String, ForgeError> {
    let repo = state
        .db
        .get_repository_by_owner_name(owner, repo_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Repository '{}/{}' not found", owner, repo_name)))?;
    Ok(repo.id)
}

pub async fn get_profile(
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let profile = state.discovery.get_user_profile(&username).await?;
    Ok(Json(profile))
}

pub async fn update_profile(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(username): Path<String>,
    Json(payload): Json<UpdateProfileRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    if user.username.to_lowercase() != username.to_lowercase() {
        return Err(ForgeError::Forbidden("Cannot edit another user's profile".to_string()));
    }

    let updated = state
        .discovery
        .update_user_profile(
            &user.id,
            payload.bio.as_deref(),
            payload.company.as_deref(),
            payload.location.as_deref(),
            payload.website_url.as_deref(),
            payload.twitter_username.as_deref(),
            payload.profile_readme.as_deref(),
            payload.pinned_repo_ids,
        )
        .await?;
    Ok(Json(updated))
}

pub async fn list_popular_topics(State(state): State<AppState>) -> Result<impl IntoResponse, ForgeError> {
    let topics = state.discovery.list_popular_topics(30).await?;
    Ok(Json(topics))
}

pub async fn list_explore_repositories(
    State(state): State<AppState>,
    Query(query): Query<ExploreQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let repos = state.discovery.list_explore_repositories(query.topic.as_deref()).await?;
    Ok(Json(repos))
}

pub async fn list_repo_topics(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let topics = state.discovery.list_repository_topics(&repo_id).await?;
    Ok(Json(topics))
}

pub async fn set_repo_topics(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<SetTopicsRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let topics = state.discovery.set_repository_topics(&user.id, &repo_id, &payload.topics).await?;
    Ok(Json(topics))
}

pub async fn list_repo_languages(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let langs = state.discovery.list_repository_languages(&repo_id).await?;
    Ok(Json(langs))
}

pub async fn analyze_repo_languages(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let langs = state.discovery.analyze_repository_languages(&repo_id).await?;
    Ok(Json(langs))
}
