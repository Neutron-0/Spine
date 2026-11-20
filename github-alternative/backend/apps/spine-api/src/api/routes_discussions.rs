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
pub struct ListDiscussionsQuery {
    pub category_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCategoryRequest {
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub emoji: Option<String>,
    pub is_answerable: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateDiscussionRequest {
    pub category_id: String,
    pub title: String,
    pub body: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateDiscussionRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub category_id: Option<String>,
    pub is_pinned: Option<bool>,
    pub is_locked: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateCommentRequest {
    pub body: String,
    pub parent_comment_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReactionRequest {
    pub reaction: String,
}

async fn resolve_repo(
    state: &AppState,
    owner: &str,
    repo_name: &str,
) -> Result<String, ForgeError> {
    let repo = state
        .db
        .get_repository_by_owner_name(owner, repo_name)
        .await?
        .ok_or_else(|| {
            ForgeError::NotFound(format!("Repository '{}/{}' not found", owner, repo_name))
        })?;
    Ok(repo.id)
}

pub async fn list_categories(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state.permissions.require_repo_read(caller_id, &repo_id).await?;
    let cats = state.discussions.list_categories(&repo_id).await?;
    Ok(Json(cats))
}

pub async fn create_category(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateCategoryRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let cat = state
        .discussions
        .create_category(
            &user.id,
            &repo_id,
            &payload.name,
            &payload.slug,
            payload.description.as_deref(),
            payload.emoji.as_deref().unwrap_or("💬"),
            payload.is_answerable.unwrap_or(false),
        )
        .await?;
    Ok(Json(cat))
}

pub async fn list_discussions(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<ListDiscussionsQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let user_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    let list = state
        .discussions
        .list_discussions(user_id, &repo_id, query.category_id.as_deref())
        .await?;
    Ok(Json(list))
}

pub async fn create_discussion(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateDiscussionRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let disc = state
        .discussions
        .create_discussion(
            &user.id,
            &repo_id,
            &payload.category_id,
            &payload.title,
            &payload.body,
        )
        .await?;
    Ok(Json(disc))
}

pub async fn get_discussion(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i32)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let user_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    let disc = state
        .discussions
        .get_discussion_by_number(user_id, &repo_id, number)
        .await?;
    Ok(Json(disc))
}

pub async fn update_discussion(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i32)>,
    Json(payload): Json<UpdateDiscussionRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let disc = state
        .discussions
        .update_discussion(
            &user.id,
            &repo_id,
            number,
            payload.title.as_deref(),
            payload.body.as_deref(),
            payload.category_id.as_deref(),
            payload.is_pinned,
            payload.is_locked,
        )
        .await?;
    Ok(Json(disc))
}

pub async fn list_comments(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i32)>,
) -> Result<impl IntoResponse, ForgeError> {
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let user_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    let comments = state
        .discussions
        .list_comments(user_id, &repo_id, number)
        .await?;
    Ok(Json(comments))
}

pub async fn create_comment(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i32)>,
    Json(payload): Json<CreateCommentRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let comment = state
        .discussions
        .create_comment(
            &user.id,
            &repo_id,
            number,
            payload.parent_comment_id.as_deref(),
            &payload.body,
        )
        .await?;
    Ok(Json(comment))
}

pub async fn mark_answer(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number, comment_id)): Path<(String, String, i32, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let disc = state
        .discussions
        .mark_answer(&user.id, &repo_id, number, &comment_id)
        .await?;
    Ok(Json(disc))
}

pub async fn toggle_comment_reaction(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number, comment_id)): Path<(String, String, i32, String)>,
    Json(payload): Json<ReactionRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    state.permissions.require_repo_read(Some(&user.id), &repo_id).await?;

    let comments = state.discussions.list_comments(Some(&user.id), &repo_id, number).await?;
    if !comments.iter().any(|c| c.id == comment_id) {
        return Err(ForgeError::NotFound(
            "Comment not found on this discussion".into(),
        ));
    }

    let added = state
        .discussions
        .toggle_reaction(&user.id, "COMMENT", &comment_id, &payload.reaction)
        .await?;
    Ok(Json(
        serde_json::json!({ "added": added, "reaction": payload.reaction }),
    ))
}
