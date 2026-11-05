use crate::api::middleware::AppState;
use crate::core::types::User;
use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use spine_migration::engine::MigrationEngine;
use spine_migration::platform::attribution::AttributionPolicy;

#[derive(Debug, Deserialize)]
pub struct InspectRequest {
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
pub struct PlanRequest {
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
pub struct RunMigrationRequest {
    pub source_path: String,
    pub target_name: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExportGitRequest {
    pub repo_name: String,
    pub target_path: String,
    #[serde(default)]
    pub is_bare: bool,
}

#[derive(Debug, Serialize)]
pub struct MigrationPlanResponse {
    pub source_path: String,
    pub detected_kind: String,
    pub stages: Vec<String>,
    pub estimated_blobs: usize,
    pub estimated_commits: usize,
}

pub async fn inspect_source(
    State(_state): State<AppState>,
    _user: Option<Extension<User>>,
    Json(req): Json<InspectRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let inspection = MigrationEngine::inspect(&req.source_path)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Inspection error: {}", e)))?;
    Ok(Json(inspection))
}

pub async fn plan_migration(
    State(_state): State<AppState>,
    _user: Option<Extension<User>>,
    Json(req): Json<PlanRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let inspection = MigrationEngine::inspect(&req.source_path)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Planning error: {}", e)))?;

    let plan = MigrationPlanResponse {
        source_path: req.source_path,
        detected_kind: format!("{:?}", inspection.source_kind),
        stages: vec![
            "Security and path traversal pre-flight checks".to_string(),
            "Blob and LFS artifact ingestion into CAS".to_string(),
            "Tree reconstruction with canonical sorting".to_string(),
            "Commit DAG ordering and author normalization".to_string(),
            "Branch and tag reference synchronization".to_string(),
            "Platform metadata hydration with safe attribution".to_string(),
            "Integrity verification & cryptographic audit".to_string(),
        ],
        estimated_blobs: inspection.commit_count * 5 + 10,
        estimated_commits: inspection.commit_count,
    };

    Ok(Json(plan))
}

pub async fn run_migration(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(req): Json<RunMigrationRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let storage_root = state.storage_root();
    let target_dir = storage_root.join("repos").join(&req.target_name);

    let mut policy = AttributionPolicy::default();
    if let Some(Extension(u)) = user {
        policy.fallback_spine_user = u.username;
    }

    let report =
        MigrationEngine::run_git_import(&req.source_path, &target_dir, policy, req.dry_run)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Migration error: {}", e),
                )
            })?;

    Ok(Json(report))
}

pub async fn export_git(
    State(state): State<AppState>,
    _user: Option<Extension<User>>,
    Json(req): Json<ExportGitRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let storage_root = state.storage_root();
    let source_spine_repo = storage_root.join("repos").join(&req.repo_name);

    if !source_spine_repo.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Repository '{}' not found", req.repo_name),
        ));
    }

    let stats = MigrationEngine::run_git_export(&source_spine_repo, &req.target_path, req.is_bare)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                format!("Git export error: {}", e),
            )
        })?;

    Ok(Json(stats))
}
