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
    pub storage_nodes: bool,
    pub storage_nodes_healthy: usize,
    pub storage_nodes_required: usize,
}

pub async fn health_liveness() -> impl IntoResponse {
    Json(HealthStatusResponse {
        status: "ok",
        service: "spine-forge-api",
        version: "0.1.0",
    })
}

pub async fn health_readiness(State(state): State<AppState>) -> impl IntoResponse {
    let db_ok = tokio::time::timeout(std::time::Duration::from_secs(2), state.db.ping())
        .await
        .map(|r| r.is_ok())
        .unwrap_or(false);

    let storage_ok = state.vcs.storage_root().exists();

    let coord_ok = match &state.coordinator {
        Some(c) => {
            let health_fut = c.check_health();
            tokio::time::timeout(std::time::Duration::from_secs(2), health_fut)
                .await
                .unwrap_or(false)
        }
        None => true, // Standalone mode without external coordinator
    };

    let mut healthy_nodes = 0usize;
    for (_id, addr) in &state.storage_nodes {
        let client = crate::ClusterNodeClient::new(addr.clone());
        let node_check = async {
            match client.query_maintenance_status().await {
                Ok((in_maint, _)) => !in_maint,
                Err(_) => false,
            }
        };
        if tokio::time::timeout(std::time::Duration::from_secs(2), node_check)
            .await
            .unwrap_or(false)
        {
            healthy_nodes += 1;
        }
    }

    let min_nodes = if state.profile == crate::core::profile::DeploymentProfile::CoreForge {
        3
    } else {
        0
    };

    let storage_nodes_ok = if state.profile == crate::core::profile::DeploymentProfile::CoreForge {
        healthy_nodes >= min_nodes && healthy_nodes == state.storage_nodes.len()
    } else if !state.storage_nodes.is_empty() {
        healthy_nodes == state.storage_nodes.len()
    } else {
        true
    };

    let overall = db_ok && storage_ok && coord_ok && storage_nodes_ok;
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
        storage_nodes: storage_nodes_ok,
        storage_nodes_healthy: healthy_nodes,
        storage_nodes_required: min_nodes,
    };

    (status_code, Json(resp))
}

#[derive(Serialize)]
pub struct DiagnosticsResponse {
    pub status: &'static str,
    pub service: &'static str,
    pub version: &'static str,
    pub profile: &'static str,
    pub untrusted_ci_allowed: bool,
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
    let db_ok = state.db.ping().await.is_ok();
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
        profile: state.profile.name(),
        untrusted_ci_allowed: state.profile.allow_untrusted_ci(),
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
