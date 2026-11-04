use axum::{
    extract::{Extension, Query, State},
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::{SearchResultType, User};

#[derive(Debug, Deserialize)]
pub struct SearchQueryParams {
    pub q: String,
    #[serde(rename = "type")]
    pub result_type: Option<String>,
    pub repo: Option<String>,
}

pub async fn search_handler(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Query(params): Query<SearchQueryParams>,
) -> Result<impl IntoResponse, ForgeError> {
    let r_type = params
        .result_type
        .as_deref()
        .and_then(|t| match t.to_lowercase().as_str() {
            "repo" | "repository" => Some(SearchResultType::Repository),
            "code" => Some(SearchResultType::Code),
            "issue" => Some(SearchResultType::Issue),
            "pr" | "pull" | "pull_request" => Some(SearchResultType::PullRequest),
            "discussion" => Some(SearchResultType::Discussion),
            _ => None,
        });

    let user_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    let response = state
        .search
        .search(&params.q, r_type, params.repo.as_deref(), user_id)
        .await?;

    Ok(Json(response))
}
