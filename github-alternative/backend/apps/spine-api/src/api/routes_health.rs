use super::middleware::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;

#[derive(Serialize)]
pub struct HealthStatusResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
}

#[derive(Serialize)]
pub struct ReadinessResponse {
    pub status: &'static str,
    pub database: bool,
    pub storage: bool,
    pub coordinator: bool,
}

pub async fn health_liveness() -> impl IntoResponse {
    Json(HealthStatusResponse {
        status: "ok",
        service: "spine-forge-api",
        version: "0.1.0",
    })
}

pub async fn health_readiness(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = state.db.get_user_by_id("readiness_check").await.is_ok();
    let storage_ok = state.vcs.storage_root().exists();
    let coord_ok = match &state.coordinator {
        Some(c) => c.check_health().await,
        None => true, // Standalone mode without external coordinator
    };

    let overall = db_ok && storage_ok && coord_ok;
    let status_code = if overall {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let resp = ReadinessResponse {
        status: if overall { "ready" } else { "not_ready" },
        database: db_ok,
        storage: storage_ok,
        coordinator: coord_ok,
    };

    (status_code, Json(resp))
}

#[derive(Serialize)]
pub struct DiagnosticsResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
    pub database_connected: bool,
    pub coordinator_connected: bool,
    pub active_runners_count: usize,
    pub pending_webhook_outbox_depth: usize,
    pub queued_workflow_jobs_count: usize,
    pub total_public_repositories: usize,
    pub timestamp_nanos: i64,
}

pub async fn health_diagnostics(State(state): State<AppState>) -> impl IntoResponse {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let db_ok = state.db.get_user_by_id("readiness_check").await.is_ok();
    let coord_ok = match &state.coordinator {
        Some(c) => c.check_health().await,
        None => true,
    };

    let all_runners = state.db.list_runners().await.unwrap_or_default();
    let active_runners = all_runners
        .into_iter()
        .filter(|r| r.status == "ACTIVE")
        .count();
    let queued_jobs = state
        .db
        .list_queued_workflow_jobs(now)
        .await
        .unwrap_or_default();
    let outbox = state
        .db
        .claim_pending_webhook_outbox(100, now)
        .await
        .unwrap_or_default();
    let public_repos = state
        .db
        .list_public_repositories()
        .await
        .unwrap_or_default();

    let resp = DiagnosticsResponse {
        status: if db_ok && coord_ok {
            "HEALTHY"
        } else {
            "DEGRADED"
        },
        service: "spine-forge-api",
        version: "0.1.0",
        database_connected: db_ok,
        coordinator_connected: coord_ok,
        active_runners_count: active_runners,
        pending_webhook_outbox_depth: outbox.len(),
        queued_workflow_jobs_count: queued_jobs.len(),
        total_public_repositories: public_repos.len(),
        timestamp_nanos: now,
    };

    let status_code = if db_ok && coord_ok {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    (status_code, Json(resp))
}
