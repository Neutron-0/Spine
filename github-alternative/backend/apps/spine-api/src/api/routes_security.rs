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
pub struct TriggerScanRequest {
    pub commit_oid: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RemediateFindingRequest {
    pub status: String, // "RESOLVED" or "FALSE_POSITIVE"
}

#[derive(Debug, Deserialize)]
pub struct RemediateVulnRequest {
    pub status: String, // "FIXED" or "DISMISSED"
}

#[derive(Debug, Deserialize)]
pub struct FilterQuery {
    pub status: Option<String>,
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

pub async fn trigger_secret_scan(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<TriggerScanRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let commit_oid = payload.commit_oid.as_deref().unwrap_or("HEAD");
    let scan = state
        .security
        .scan_repository_secrets(&user.id, &repo_id, commit_oid)
        .await?;
    Ok(Json(scan))
}

pub async fn list_secret_findings(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<FilterQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let findings = state
        .security
        .list_secret_findings(&user.id, &repo_id, query.status.as_deref())
        .await?;
    Ok(Json(findings))
}

pub async fn remediate_secret_finding(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, id)): Path<(String, String, String)>,
    Json(payload): Json<RemediateFindingRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    state
        .security
        .remediate_secret_finding(&user.id, &repo_id, &id, &payload.status)
        .await?;
    Ok(Json(
        serde_json::json!({ "id": id, "status": payload.status }),
    ))
}

pub async fn trigger_dependency_audit(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<TriggerScanRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let commit_oid = payload.commit_oid.as_deref().unwrap_or("HEAD");
    let scan = state
        .security
        .audit_dependencies(&user.id, &repo_id, commit_oid)
        .await?;
    Ok(Json(scan))
}

pub async fn list_dependencies(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let deps = state.security.list_dependencies(&user.id, &repo_id).await?;
    Ok(Json(deps))
}

pub async fn list_vulnerabilities(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<FilterQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let vulns = state
        .security
        .list_vulnerabilities(&user.id, &repo_id, query.status.as_deref())
        .await?;
    Ok(Json(vulns))
}

pub async fn remediate_vulnerability(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, id)): Path<(String, String, String)>,
    Json(payload): Json<RemediateVulnRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    state
        .security
        .remediate_vulnerability(&user.id, &repo_id, &id, &payload.status)
        .await?;
    Ok(Json(
        serde_json::json!({ "id": id, "status": payload.status }),
    ))
}

pub async fn export_sbom_spdx(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let spdx = state.security.export_sbom_spdx(&user.id, &repo_id).await?;
    Ok(Json(spdx))
}

pub async fn export_sbom_cyclonedx(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let cdx = state
        .security
        .export_sbom_cyclonedx(&user.id, &repo_id)
        .await?;
    Ok(Json(cdx))
}

pub async fn get_security_summary(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(user) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo_id = resolve_repo(&state, &owner, &repo_name).await?;
    let summary = state
        .security
        .get_security_summary(&user.id, &repo_id)
        .await?;
    Ok(Json(summary))
}

pub async fn get_advisory_provenance(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ForgeError> {
    let prov = state.security.get_advisory_feed_provenance().await?;
    Ok(Json(prov))
}
