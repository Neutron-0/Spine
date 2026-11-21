use super::middleware::AppState;
use crate::auth::pat::hash_pat;
use crate::auth::session::hash_session_token;
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
/// Supports:
/// 1. Direct configured SPINE_ADMIN_TOKEN or SPINE_ADMIN_TOKEN_FILE match.
/// 2. Valid authenticated User session or PAT whose username is 'admin' or has administrative rights.
async fn verify_admin_auth(
    state: &AppState,
    headers: &HeaderMap,
    caller: Option<&User>,
) -> Result<(), ForgeError> {
    let auth_header = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim());

    let raw_token = auth_header
        .and_then(|h| h.strip_prefix("Bearer ").or(Some(h)))
        .map(|s| s.trim())
        .unwrap_or("");

    // 1. Check direct SPINE_ADMIN_TOKEN / SPINE_ADMIN_TOKEN_FILE
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

    // 2. Check resolved user from auth_middleware
    if let Some(user) = caller {
        if user.username.to_lowercase().contains("admin") {
            return Ok(());
        }
    }

    // 3. Fallback: check raw_token against database PATs or Sessions directly if not already resolved
    if !raw_token.is_empty() {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        if raw_token.starts_with("sp_pat_") || raw_token.starts_with("spine_pat_") {
            let p_hash = hash_pat(raw_token);
            if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
                if !pat.revoked && pat.expires_at_nanos >= now {
                    if pat.scopes.iter().any(|s| s == "admin" || s == "*") {
                        return Ok(());
                    }
                    if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                        if u.username.to_lowercase().contains("admin") {
                            return Ok(());
                        }
                    }
                }
            }
        } else {
            let s_hash = hash_session_token(raw_token);
            if let Ok(Some(sess)) = state.db.get_session_by_hash(&s_hash).await {
                if !sess.revoked && sess.expires_at_nanos >= now {
                    if let Ok(Some(u)) = state.db.get_user_by_id(&sess.user_id).await {
                        if u.username.to_lowercase().contains("admin") {
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    Err(ForgeError::Unauthorized(
        "Administrative authorization required".into(),
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

    // Propagate generation and maintenance state to consensus coordinator
    if let Some(coord) = &state.coordinator {
        if let Err(e) = coord.enter_maintenance(fencing_generation).await {
            tracing::error!(
                "Coordinator failed to enter maintenance at generation {}: {}",
                fencing_generation,
                e
            );
            // Release local barrier before failing
            let _ = guard.release().await;
            return Err(ForgeError::Internal(format!(
                "Coordinator failed to enter maintenance: {}",
                e
            )));
        }
    }

    // Propagate generation and maintenance state to storage nodes
    for (node_id, node_addr) in &state.storage_nodes {
        let client = crate::ClusterNodeClient::new(node_addr.clone());
        if let Err(e) = client.enter_maintenance(fencing_generation).await {
            tracing::error!(
                "Storage node {} ({}) failed to enter maintenance: {}",
                node_id,
                node_addr,
                e
            );
            if let Some(coord) = &state.coordinator {
                let _ = coord.exit_maintenance(fencing_generation).await;
            }
            let _ = guard.release().await;
            return Err(ForgeError::Internal(format!(
                "Storage node {} failed to enter maintenance: {}",
                node_id, e
            )));
        }
    }

    // Synchronize generation token to database and storage root so cluster participants remain unanimous
    if let Some(pool) = state.consistency_barrier.pool() {
        if let Err(e) = crate::backup::journal::RestoreJournal::record_db_generation(
            pool,
            &fencing_generation.to_string(),
            &owner_token,
        )
        .await
        {
            tracing::warn!("Failed to record advanced fencing generation in _restore_generation: {}", e);
        }
    }
    if let Err(e) = crate::backup::journal::RestoreJournal::write_storage_generation(
        state.storage_root(),
        &fencing_generation.to_string(),
    ) {
        tracing::warn!("Failed to record advanced fencing generation in storage root: {}", e);
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

pub async fn release_maintenance(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.map(|Extension(u)| u);
    verify_admin_auth(&state, &headers, caller.as_ref()).await?;

    let mut active_lock = state.active_maintenance.lock().await;
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

    // Propagate release to consensus coordinator
    if let Some(coord) = &state.coordinator {
        if let Err(e) = coord.exit_maintenance(fencing_gen).await {
            tracing::warn!(
                "Coordinator failed to exit maintenance at generation {}: {}",
                fencing_gen,
                e
            );
        }
    }

    // Propagate release to storage nodes
    for (node_id, node_addr) in &state.storage_nodes {
        let client = crate::ClusterNodeClient::new(node_addr.clone());
        if let Err(e) = client.exit_maintenance(fencing_gen).await {
            tracing::warn!(
                "Storage node {} ({}) failed to exit maintenance: {}",
                node_id,
                node_addr,
                e
            );
        }
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
