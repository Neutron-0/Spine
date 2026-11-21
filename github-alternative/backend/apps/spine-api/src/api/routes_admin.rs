use super::middleware::AppState;
use crate::auth::pat::hash_pat;
use crate::core::error::ForgeError;
use crate::core::types::User;
use axum::{
    extract::{Extension, State},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Deserialize, Debug, Default)]
pub struct AcquireMaintenanceRequest {
    pub reason: Option<String>,
    pub timeout_seconds: Option<u64>,
}

#[derive(Serialize, Debug)]
pub struct AcquireMaintenanceResponse {
    pub status: &'static str,
    pub fencing_generation: u64,
    pub owner_token: String,
    pub lease_expires_at_nanos: i64,
}

#[derive(Serialize, Debug)]
pub struct ReleaseMaintenanceResponse {
    pub status: &'static str,
}

/// Helper to authenticate an administrative caller.
/// Requires either:
/// 1. Direct configured SPINE_ADMIN_TOKEN or SPINE_ADMIN_TOKEN_FILE match.
/// 2. Valid active Personal Access Token (PAT) with explicit "admin", "system:admin", or "*" scope.
///
/// Plain usernames (such as "admin", "admin-dev", or "notadmin") do NOT grant administrative access.
async fn verify_admin_auth(
    state: &AppState,
    headers: &HeaderMap,
    _caller: Option<&User>,
) -> Result<(), ForgeError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim());

    let raw_token = auth_header
        .and_then(|h| h.strip_prefix("Bearer ").or(Some(h)))
        .map(|s| s.trim())
        .unwrap_or("");

    // 1. Check direct configured SPINE_ADMIN_TOKEN / SPINE_ADMIN_TOKEN_FILE
    let env_token = std::env::var("SPINE_ADMIN_TOKEN")
        .or_else(|_| std::env::var("ADMIN_TOKEN"))
        .ok();
    let file_token = std::env::var("SPINE_ADMIN_TOKEN_FILE")
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .map(|s| s.trim().to_string());

    if let Some(ref expected) = env_token.or(file_token) {
        if !raw_token.is_empty() && raw_token == expected {
            return Ok(());
        }
    }

    // 2. Check database Personal Access Token with explicit admin scope
    if !raw_token.is_empty() && (raw_token.starts_with("sp_pat_") || raw_token.starts_with("spine_pat_")) {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let p_hash = hash_pat(raw_token);
        if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
            if !pat.revoked && pat.expires_at_nanos >= now {
                let has_admin_scope = pat
                    .scopes
                    .iter()
                    .any(|s| s == "admin" || s == "system:admin" || s == "*");
                if has_admin_scope {
                    if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                        if u.account_status.to_uppercase() == "ACTIVE" {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    Err(ForgeError::Unauthorized(
        "Administrative authorization required: valid admin credential or PAT with 'admin' scope required"
            .into(),
    ))
}

pub async fn acquire_maintenance(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    headers: HeaderMap,
    payload: Option<Json<AcquireMaintenanceRequest>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    verify_admin_auth(&state, &headers, caller.as_ref()).await?;

    let req = payload.map(|Json(p)| p).unwrap_or_default();
    let lease_ttl_secs = req.timeout_seconds.unwrap_or(300);
    let lease_ttl = Duration::from_secs(lease_ttl_secs);
    let drain_timeout = Duration::from_secs(30);

    let mut active_lock = state.active_maintenance.lock().await;
    if active_lock.is_some() {
        return Err(ForgeError::Conflict(
            "Cluster maintenance barrier is already held by an active session".into(),
        ));
    }

    tracing::info!(
        reason = req.reason.as_deref().unwrap_or("unspecified"),
        timeout_seconds = lease_ttl_secs,
        "Acquiring cluster maintenance barrier"
    );

    let guard = state
        .consistency_barrier
        .enter_maintenance_with_lease(drain_timeout, lease_ttl)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to acquire maintenance barrier: {}", e))
        })?;

    let fencing_generation = guard.fencing_generation();
    let owner_token = guard.owner_token().to_string();
    let lease_expires_at_nanos = guard.current_lease_expiry();

    // 1. Propagate generation and maintenance state to consensus coordinator
    let mut coordinator_entered = false;
    if let Some(coord) = &state.coordinator {
        if let Err(e) = coord.enter_maintenance(fencing_generation).await {
            tracing::error!(
                "Coordinator failed to enter maintenance at generation {}: {}",
                fencing_generation,
                e
            );
            let _ = guard.release().await;
            return Err(ForgeError::Internal(format!(
                "Coordinator failed to enter maintenance: {}",
                e
            )));
        }
        coordinator_entered = true;
    }

    // 2. Propagate generation and maintenance state to storage nodes, tracking entered nodes for rollback
    let mut entered_storage_nodes: Vec<(String, String)> = Vec::new();
    for (node_id, node_addr) in &state.storage_nodes {
        let client = crate::ClusterNodeClient::new(node_addr.clone());
        if let Err(e) = client.enter_maintenance(fencing_generation).await {
            tracing::error!(
                "Storage node {} ({}) failed to enter maintenance: {}. Compensating entered participants...",
                node_id,
                node_addr,
                e
            );
            // Roll back all previously entered storage nodes
            for (rollback_id, rollback_addr) in entered_storage_nodes.iter().rev() {
                let rb_client = crate::ClusterNodeClient::new(rollback_addr.clone());
                if let Err(rb_err) = rb_client.exit_maintenance(fencing_generation).await {
                    tracing::error!(
                        "Failed to compensate/exit storage node {} ({}) during rollback: {}",
                        rollback_id, rollback_addr, rb_err
                    );
                }
            }
            // Roll back coordinator
            if coordinator_entered {
                if let Some(coord) = &state.coordinator {
                    let _ = coord.exit_maintenance(fencing_generation).await;
                }
            }
            let _ = guard.release().await;
            return Err(ForgeError::Internal(format!(
                "Storage node {} failed to enter maintenance: {}",
                node_id, e
            )));
        }
        entered_storage_nodes.push((node_id.clone(), node_addr.clone()));
    }

    // 3. Synchronize generation token to database and storage root so cluster participants remain unanimous.
    // If generation persistence fails, fail closed: compensate all entered participants and abort acquisition.
    if let Some(pool) = state.consistency_barrier.pool() {
        if let Err(e) = crate::backup::journal::RestoreJournal::record_db_generation(
            pool,
            &fencing_generation.to_string(),
            &owner_token,
        )
        .await
        {
            tracing::error!(
                "Failed to record advanced fencing generation in database: {}. Compensating entered participants...",
                e
            );
            for (_rollback_id, rollback_addr) in entered_storage_nodes.iter().rev() {
                let rb_client = crate::ClusterNodeClient::new(rollback_addr.clone());
                let _ = rb_client.exit_maintenance(fencing_generation).await;
            }
            if coordinator_entered {
                if let Some(coord) = &state.coordinator {
                    let _ = coord.exit_maintenance(fencing_generation).await;
                }
            }
            let _ = guard.release().await;
            return Err(ForgeError::Internal(format!(
                "Database fencing generation persistence failed: {}",
                e
            )));
        }
    }
    if let Err(e) = crate::backup::journal::RestoreJournal::write_storage_generation(
        state.storage_root(),
        &fencing_generation.to_string(),
    ) {
        tracing::error!(
            "Failed to record advanced fencing generation in storage root: {}. Compensating entered participants...",
            e
        );
        for (_rollback_id, rollback_addr) in entered_storage_nodes.iter().rev() {
            let rb_client = crate::ClusterNodeClient::new(rollback_addr.clone());
            let _ = rb_client.exit_maintenance(fencing_generation).await;
        }
        if coordinator_entered {
            if let Some(coord) = &state.coordinator {
                let _ = coord.exit_maintenance(fencing_generation).await;
            }
        }
        let _ = guard.release().await;
        return Err(ForgeError::Internal(format!(
            "Storage root fencing generation persistence failed: {}",
            e
        )));
    }

    *active_lock = Some(guard);

    let resp = AcquireMaintenanceResponse {
        status: "acquired",
        fencing_generation,
        owner_token,
        lease_expires_at_nanos,
    };

    Ok((StatusCode::OK, Json(resp)))
}

#[derive(Debug, Deserialize, Default)]
pub struct ReleaseMaintenanceRequest {
    pub owner_token: Option<String>,
}

pub async fn release_maintenance(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    headers: HeaderMap,
    payload: Option<Json<ReleaseMaintenanceRequest>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    verify_admin_auth(&state, &headers, caller.as_ref()).await?;

    let req = payload.map(|Json(p)| p).unwrap_or_default();
    let provided_owner_token = req
        .owner_token
        .as_deref()
        .or_else(|| {
            headers
                .get("x-spine-owner-token")
                .or_else(|| headers.get("x-owner-token"))
                .and_then(|v| v.to_str().ok())
        });

    let mut active_lock = state.active_maintenance.lock().await;

    // If an active guard is held, require a matching owner token.
    // Reject mismatched or missing tokens without taking or releasing the guard.
    if let Some(guard) = active_lock.as_ref() {
        let expected_owner_token = guard.owner_token();
        match provided_owner_token {
            Some(token) if token == expected_owner_token => {
                // Owner token matches; proceed to release below
            }
            Some(_) => {
                tracing::warn!(
                    "Rejected maintenance release: mismatched owner token. Active barrier remains held."
                );
                return Err(ForgeError::Forbidden(
                    "Mismatched maintenance barrier owner token. Only the acquiring operation can release this barrier.".into(),
                ));
            }
            None => {
                tracing::warn!(
                    "Rejected maintenance release: missing required owner token for active barrier. Active barrier remains held."
                );
                return Err(ForgeError::Forbidden(
                    "Missing required maintenance barrier owner token. An active barrier requires ownership verification to release.".into(),
                ));
            }
        }
    }

    let guard_opt = active_lock.take();

    let fencing_gen = if let Some(guard) = guard_opt {
        let gen = guard.fencing_generation();
        guard.release().await.map_err(|e| {
            ForgeError::Internal(format!("Failed to release maintenance barrier: {}", e))
        })?;
        gen
    } else {
        // Fallback: clear barrier if unheld
        state
            .consistency_barrier
            .clear_barrier_if_unheld()
            .await
            .ok();
        state.consistency_barrier.current_fencing_generation()
    };

    let mut participant_errors = Vec::new();

    // Propagate release to consensus coordinator
    if let Some(coord) = &state.coordinator {
        if let Err(e) = coord.exit_maintenance(fencing_gen).await {
            tracing::error!(
                "Coordinator failed to exit maintenance at generation {}: {}",
                fencing_gen,
                e
            );
            participant_errors.push(format!("Coordinator exit failed: {}", e));
        }
    }

    // Propagate release to storage nodes
    for (node_id, node_addr) in &state.storage_nodes {
        let client = crate::ClusterNodeClient::new(node_addr.clone());
        if let Err(e) = client.exit_maintenance(fencing_gen).await {
            tracing::error!(
                "Storage node {} ({}) failed to exit maintenance: {}",
                node_id,
                node_addr,
                e
            );
            participant_errors.push(format!("Storage node {} exit failed: {}", node_id, e));
        }
    }

    if !participant_errors.is_empty() {
        return Err(ForgeError::ServiceUnavailable(format!(
            "Cluster maintenance release failed or degraded: participants failed to exit maintenance [{}]",
            participant_errors.join("; ")
        )));
    }

    tracing::info!(
        fencing_generation = fencing_gen,
        "Cluster maintenance barrier released"
    );

    let resp = ReleaseMaintenanceResponse {
        status: "released",
    };

    Ok((StatusCode::OK, Json(resp)))
}
