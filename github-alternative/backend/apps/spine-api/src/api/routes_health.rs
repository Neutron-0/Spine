use super::middleware::AppState;
use crate::backup::journal::RestoreJournal;
use crate::backup::RestorePhase;
use crate::core::profile::DeploymentProfile;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use serde::Serialize;
use std::collections::HashSet;

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
    pub cluster_consistent: bool,
    pub restore_in_progress: bool,
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

    let storage_root = state.vcs.storage_root();
    let storage_ok = storage_root.exists();

    // Check for active or incomplete restore journals
    let restore_in_progress = match RestoreJournal::load(storage_root) {
        Ok(Some(journal)) => !matches!(
            journal.phase,
            RestorePhase::Committed | RestorePhase::Completed
        ),
        _ => false,
    };

    let mut coord_gen: Option<u64> = None;
    let coord_ok = match &state.coordinator {
        Some(c) => {
            let health_fut = async {
                match c.query_maintenance_status().await {
                    Ok((in_maint, gen)) => {
                        if in_maint {
                            false
                        } else {
                            coord_gen = Some(gen);
                            true
                        }
                    }
                    Err(_) => false,
                }
            };
            tokio::time::timeout(std::time::Duration::from_secs(2), health_fut)
                .await
                .unwrap_or(false)
        }
        None => true, // Standalone mode without external coordinator
    };

    // Canonical node configuration checks
    let mut seen_ids = HashSet::new();
    let mut seen_addrs = HashSet::new();
    let mut duplicate_found = false;
    for (id, addr) in &state.storage_nodes {
        if !seen_ids.insert(id.clone()) || !seen_addrs.insert(addr.clone()) {
            duplicate_found = true;
            break;
        }
    }

    let min_nodes = if state.profile == DeploymentProfile::CoreForge {
        3
    } else {
        0
    };

    let required_ids_present = if state.profile == DeploymentProfile::CoreForge {
        seen_ids.contains("node-1") && seen_ids.contains("node-2") && seen_ids.contains("node-3")
    } else {
        true
    };

    let mut healthy_nodes = 0usize;
    let mut node_generations = Vec::new();
    if !duplicate_found && required_ids_present {
        for (expected_id, addr) in &state.storage_nodes {
            let client = crate::ClusterNodeClient::new(addr.clone());
            let node_check = async {
                match client.query_node_details().await {
                    Ok((in_maint, gen, _, responding_id)) => {
                        if in_maint {
                            return None;
                        }
                        if let Some(ref r_id) = responding_id {
                            if r_id != expected_id {
                                return None;
                            }
                        }
                        Some(gen)
                    }
                    Err(_) => None,
                }
            };
            if let Ok(Some(gen)) =
                tokio::time::timeout(std::time::Duration::from_secs(2), node_check).await
            {
                healthy_nodes += 1;
                node_generations.push(gen);
            }
        }
    }

    let storage_nodes_ok = if state.profile == DeploymentProfile::CoreForge {
        !duplicate_found
            && required_ids_present
            && healthy_nodes >= min_nodes
            && healthy_nodes == state.storage_nodes.len()
    } else if !state.storage_nodes.is_empty() {
        !duplicate_found && healthy_nodes == state.storage_nodes.len()
    } else {
        true
    };

    // Check generation consistency across all responsive cluster participants
    let mut cluster_consistent = true;
    if let Some(c_gen) = coord_gen {
        for n_gen in &node_generations {
            if *n_gen != c_gen {
                cluster_consistent = false;
                break;
            }
        }
    } else if !node_generations.is_empty() {
        let first_gen = node_generations[0];
        for n_gen in &node_generations {
            if *n_gen != first_gen {
                cluster_consistent = false;
                break;
            }
        }
    }

    let overall = db_ok
        && storage_ok
        && coord_ok
        && storage_nodes_ok
        && cluster_consistent
        && !restore_in_progress;

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
        cluster_consistent,
        restore_in_progress,
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
