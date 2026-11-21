use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::User;

#[derive(Deserialize)]
pub struct DispatchWorkflowRequest {
    pub ref_name: Option<String>,
    pub inputs: Option<serde_json::Value>,
}

#[derive(Deserialize)]
pub struct SyncWorkflowsRequest {
    pub files: Vec<WorkflowFileInput>,
}

#[derive(Deserialize)]
pub struct WorkflowFileInput {
    pub path: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct RunsQuery {
    pub limit: Option<i64>,
}

pub async fn list_workflows(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;
    let defs = state.db.list_workflow_definitions(&repo.id).await?;
    Ok(Json(defs))
}

pub async fn sync_workflows(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<SyncWorkflowsRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let file_tuples: Vec<(&str, &str)> = payload
        .files
        .iter()
        .map(|f| (f.path.as_str(), f.content.as_str()))
        .collect();

    let synced = state
        .workflows
        .sync_workflows_from_tree(&repo.id, file_tuples)
        .await?;
    Ok((StatusCode::CREATED, Json(synced)))
}

pub async fn dispatch_workflow(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, _workflow_id)): Path<(String, String, String)>,
    Json(payload): Json<DispatchWorkflowRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let ref_name = payload
        .ref_name
        .unwrap_or_else(|| repo.default_branch.clone());
    let commit_oid = state
        .vcs
        .list_branches(&repo.native_repo_id)
        .ok()
        .and_then(|branches| {
            branches
                .into_iter()
                .find(|b| b.name == ref_name)
                .map(|b| b.commit_id)
        })
        .unwrap_or_else(|| "0000000000000000000000000000000000000000".to_string());

    let runs = state
        .workflows
        .trigger_workflows_with_provenance(
            &repo.id,
            Some(&repo.id),
            &commit_oid,
            &ref_name,
            "workflow_dispatch",
            &caller.id,
        )
        .await?;

    Ok((StatusCode::CREATED, Json(runs)))
}

pub async fn list_runs(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<RunsQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;
    let limit = query.limit.unwrap_or(50);
    let runs = state.workflows.list_runs(&repo.id, limit).await?;
    Ok(Json(runs))
}

pub async fn get_run(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let run = state
        .workflows
        .get_run(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Workflow run not found".into()))?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state
        .permissions
        .require_repo_read(caller_id, &run.repo_id)
        .await?;
    Ok(Json(run))
}

pub async fn cancel_run(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let run = state
        .workflows
        .get_run(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Workflow run not found".into()))?;

    let repo = state
        .db
        .get_repository_by_id(&run.repo_id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Repository not found".into()))?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    state.workflows.cancel_run(&id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_jobs(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let run = state
        .workflows
        .get_run(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Workflow run not found".into()))?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state
        .permissions
        .require_repo_read(caller_id, &run.repo_id)
        .await?;
    let jobs = state.workflows.list_jobs(&id).await?;
    Ok(Json(jobs))
}

pub async fn get_job(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let job = state
        .workflows
        .get_job(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Job not found".into()))?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state
        .permissions
        .require_repo_read(caller_id, &job.repo_id)
        .await?;
    Ok(Json(job))
}

pub async fn list_steps(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let job = state
        .workflows
        .get_job(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Job not found".into()))?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state
        .permissions
        .require_repo_read(caller_id, &job.repo_id)
        .await?;
    let steps = state.workflows.list_steps(&id).await?;
    Ok(Json(steps))
}

pub async fn list_logs(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let job = state
        .workflows
        .get_job(&id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Job not found".into()))?;
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    state
        .permissions
        .require_repo_read(caller_id, &job.repo_id)
        .await?;
    let logs = state.workflows.list_logs(&id).await?;
    Ok(Json(logs))
}
