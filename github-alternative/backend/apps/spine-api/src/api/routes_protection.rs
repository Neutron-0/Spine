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
pub struct SetBranchProtectionRequest {
    pub branch_pattern: String,
    pub require_pull_request: Option<bool>,
    pub required_approvals: Option<i32>,
    pub prevent_self_approval: Option<bool>,
    pub require_changes_addressed: Option<bool>,
    pub dismiss_stale_reviews_on_push: Option<bool>,
    pub restrict_merges_to_admins: Option<bool>,
    pub required_status_checks: Option<Vec<String>>,
}

pub async fn list_branch_protection(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let rules = state.db.list_branch_protection_rules(&repo.id).await?;
    Ok(Json(rules))
}

pub async fn set_branch_protection(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<SetBranchProtectionRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_admin_access(Some(&caller), &repo)
        .await?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let rule = BranchProtectionRule {
        id: format!("bpr_{}", uuid::Uuid::new_v4().simple()),
        repo_id: repo.id.clone(),
        branch_pattern: payload.branch_pattern.trim().to_string(),
        require_pull_request: payload.require_pull_request.unwrap_or(true),
        required_approvals: payload.required_approvals.unwrap_or(1),
        prevent_self_approval: payload.prevent_self_approval.unwrap_or(true),
        require_changes_addressed: payload.require_changes_addressed.unwrap_or(true),
        dismiss_stale_reviews_on_push: payload.dismiss_stale_reviews_on_push.unwrap_or(true),
        restrict_merges_to_admins: payload.restrict_merges_to_admins.unwrap_or(false),
        required_status_checks: payload.required_status_checks.unwrap_or_default(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let saved = state.db.set_branch_protection(rule).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

pub async fn delete_branch_protection(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, rule_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_admin_access(Some(&caller), &repo)
        .await?;

    let rules = state.db.list_branch_protection_rules(&repo.id).await?;
    if !rules.iter().any(|r| r.id == rule_id) {
        return Err(ForgeError::NotFound(
            "Branch protection rule not found in repository".into(),
        ));
    }

    state.db.delete_branch_protection(&rule_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
