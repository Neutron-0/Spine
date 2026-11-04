use crate::artifact::ArtifactService;
use crate::auth::mail::MailDelivery;
use crate::auth::pat::hash_pat;
use crate::auth::rate_limit::RateLimiter;
use crate::auth::session::hash_session_token;
use crate::coordinator::CoordinatorClient;
use crate::core::types::*;
use crate::db::Database;
use crate::packages::{CargoRegistryAdapter, PackageService};
use crate::permissions::PermissionService;
use crate::releases::ReleaseService;
use crate::repository::service::RepositoryService;
use crate::storage::vcs::NativeVcsService;
use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::Response,
};
use std::sync::Arc;

#[derive(Clone)]
pub struct AppState {
    pub db: Arc<dyn Database>,
    pub permissions: Arc<PermissionService>,
    pub repositories: Arc<RepositoryService>,
    pub vcs: Arc<NativeVcsService>,
    pub rate_limiter: Arc<RateLimiter>,
    pub mail: Arc<dyn MailDelivery>,
    pub coordinator: Option<CoordinatorClient>,
    pub artifacts: Arc<ArtifactService>,
    pub releases: Arc<ReleaseService>,
    pub packages: Arc<PackageService>,
    pub cargo: Arc<CargoRegistryAdapter>,
}

pub async fn auth_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let mut resolved_user: Option<User> = None;
    let mut resolved_session: Option<UserSession> = None;

    // 1. Check Authorization header: Bearer <token>
    let auth_header = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim());

    let pat_header = req
        .headers()
        .get("X-Spine-Token")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim());

    // 2. Check Cookie header: spine_session=<token>
    let cookie_token = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|cookies| {
            cookies.split(';').find_map(|c| {
                let trimmed = c.trim();
                if let Some(tok) = trimmed.strip_prefix("spine_session=") {
                    Some(tok.trim().to_string())
                } else {
                    None
                }
            })
        });

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    if let Some(raw_auth) = auth_header {
        let token_str = raw_auth.strip_prefix("Bearer ").unwrap_or(raw_auth).trim();
        if token_str.starts_with("sp_pat_") || token_str.starts_with("spine_pat_") {
            let p_hash = hash_pat(token_str);
            if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
                if !pat.revoked && pat.expires_at_nanos >= now {
                    let _ = state.db.update_pat_last_used(&pat.id, now).await;
                    if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                        resolved_user = Some(u);
                    }
                }
            }
        } else {
            let s_hash = hash_session_token(token_str);
            if let Ok(Some(sess)) = state.db.get_session_by_hash(&s_hash).await {
                if !sess.revoked && sess.expires_at_nanos >= now {
                    if let Ok(Some(u)) = state.db.get_user_by_id(&sess.user_id).await {
                        resolved_user = Some(u);
                        resolved_session = Some(sess);
                    }
                }
            }
        }
    } else if let Some(pat_token) = pat_header {
        let p_hash = hash_pat(pat_token);
        if let Ok(Some(pat)) = state.db.get_pat_by_hash(&p_hash).await {
            if !pat.revoked && pat.expires_at_nanos >= now {
                let _ = state.db.update_pat_last_used(&pat.id, now).await;
                if let Ok(Some(u)) = state.db.get_user_by_id(&pat.user_id).await {
                    resolved_user = Some(u);
                }
            }
        }
    } else if let Some(session_token) = cookie_token {
        let s_hash = hash_session_token(&session_token);
        if let Ok(Some(sess)) = state.db.get_session_by_hash(&s_hash).await {
            if !sess.revoked && sess.expires_at_nanos >= now {
                if let Ok(Some(u)) = state.db.get_user_by_id(&sess.user_id).await {
                    resolved_user = Some(u);
                    resolved_session = Some(sess);
                }
            }
        }
    }

    if let Some(user) = resolved_user {
        req.extensions_mut().insert(user);
    }
    if let Some(sess) = resolved_session {
        req.extensions_mut().insert(sess);
    }

    next.run(req).await
}

pub async fn request_id_middleware(req: Request, next: Next) -> Response {
    let req_id = req
        .headers()
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        header::HeaderName::from_static("x-request-id"),
        header::HeaderValue::from_str(&req_id).unwrap(),
    );
    resp
}
