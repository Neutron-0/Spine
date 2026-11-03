use super::middleware::AppState;
use crate::core::error::ForgeError;
use axum::{
    extract::{Extension, Path, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct ActivityQuery {
    pub limit: Option<usize>,
}

pub async fn list_repo_activity(
    State(state): State<AppState>,
    user: Option<Extension<crate::core::types::User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<ActivityQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state.repositories.get_repository(caller, &owner, &repo_name).await?;

    let limit = query.limit.unwrap_or(30).min(100);
    let events = state.db.list_activity_events_for_repo(&repo.id, limit).await?;
    Ok(Json(events))
}
