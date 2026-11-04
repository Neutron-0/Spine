use axum::{
    extract::{Extension, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::User;

#[derive(Deserialize)]
pub struct RegisterRunnerRequest {
    pub name: String,
    pub hostname: String,
    pub os: String,
    pub arch: String,
    pub labels: Vec<String>,
    pub max_concurrency: Option<i32>,
}

#[derive(Serialize)]
pub struct RegisterRunnerResponse {
    pub id: String,
    pub name: String,
    pub token: String,
    pub os: String,
    pub arch: String,
    pub labels: Vec<String>,
}

#[derive(Deserialize)]
pub struct HeartbeatRequest {
    pub runner_id: String,
    pub active_jobs: i32,
}

#[derive(Deserialize)]
pub struct AcquireJobRequest {
    pub runner_id: String,
    pub labels: Vec<String>,
}

#[derive(Serialize)]
pub struct AcquiredJobResponse {
    pub job: crate::core::types::WorkflowJob,
    pub steps: Vec<crate::core::types::WorkflowStep>,
    pub secrets: Vec<crate::core::types::WorkflowSecret>,
}

#[derive(Deserialize)]
pub struct IngestLogRequest {
    pub runner_id: String,
    pub fencing_token: i64,
    pub step_index: i32,
    pub stream: String,
    pub content: String,
}

#[derive(Deserialize)]
pub struct CompleteJobRequest {
    pub runner_id: String,
    pub fencing_token: i64,
    pub exit_code: i32,
    pub step_results: Vec<(i32, i32)>, // (step_index, exit_code)
}

fn extract_runner_auth(headers: &HeaderMap) -> Result<(&str, &str), ForgeError> {
    let runner_id = headers
        .get("X-Spine-Runner-Id")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ForgeError::Unauthorized("Missing X-Spine-Runner-Id header".into()))?;

    let token = headers
        .get("X-Spine-Runner-Token")
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| ForgeError::Unauthorized("Missing X-Spine-Runner-Token header".into()))?;

    Ok((runner_id, token))
}

pub async fn register_runner(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(payload): Json<RegisterRunnerRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(_caller) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;

    let (runner, raw_token) = state
        .runners
        .register_runner(
            &payload.name,
            &payload.hostname,
            &payload.os,
            &payload.arch,
            payload.labels,
            payload.max_concurrency.unwrap_or(2),
        )
        .await?;

    let resp = RegisterRunnerResponse {
        id: runner.id,
        name: runner.name,
        token: raw_token,
        os: runner.os,
        arch: runner.arch,
        labels: runner.labels,
    };

    Ok((StatusCode::CREATED, Json(resp)))
}

pub async fn list_runners(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
) -> Result<impl IntoResponse, ForgeError> {
    let Extension(_caller) =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;

    let runners = state.db.list_runners().await?;
    Ok(Json(runners))
}

pub async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<HeartbeatRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let (runner_id, token) = extract_runner_auth(&headers)?;
    if runner_id != payload.runner_id {
        return Err(ForgeError::Unauthorized("Runner ID mismatch".into()));
    }

    state.runners.authenticate_runner(runner_id, token).await?;
    state
        .runners
        .heartbeat(runner_id, payload.active_jobs)
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn acquire_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AcquireJobRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let (runner_id, token) = extract_runner_auth(&headers)?;
    if runner_id != payload.runner_id {
        return Err(ForgeError::Unauthorized("Runner ID mismatch".into()));
    }

    state.runners.authenticate_runner(runner_id, token).await?;
    let acquired = state
        .runners
        .acquire_job(runner_id, &payload.labels)
        .await?;

    match acquired {
        Some((job, steps, secrets)) => Ok(Json(Some(AcquiredJobResponse {
            job,
            steps,
            secrets,
        }))),
        None => Ok(Json(None)),
    }
}

pub async fn ingest_logs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(payload): Json<IngestLogRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let (runner_id, token) = extract_runner_auth(&headers)?;
    if runner_id != payload.runner_id {
        return Err(ForgeError::Unauthorized("Runner ID mismatch".into()));
    }

    state.runners.authenticate_runner(runner_id, token).await?;
    state
        .runners
        .ingest_logs(
            runner_id,
            &job_id,
            payload.fencing_token,
            payload.step_index,
            &payload.stream,
            &payload.content,
        )
        .await?;

    Ok(StatusCode::ACCEPTED)
}

pub async fn complete_job(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(job_id): Path<String>,
    Json(payload): Json<CompleteJobRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let (runner_id, token) = extract_runner_auth(&headers)?;
    if runner_id != payload.runner_id {
        return Err(ForgeError::Unauthorized("Runner ID mismatch".into()));
    }

    state.runners.authenticate_runner(runner_id, token).await?;
    state
        .runners
        .complete_job(
            runner_id,
            &job_id,
            payload.fencing_token,
            payload.exit_code,
            &payload.step_results,
        )
        .await?;

    Ok(StatusCode::OK)
}
