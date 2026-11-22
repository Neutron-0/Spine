use crate::artifact::ArtifactService;
use crate::auth::mail::MailDelivery;
use crate::auth::pat::hash_pat;
use crate::auth::rate_limit::RateLimiter;
use crate::auth::session::hash_session_token;
use crate::coordinator::CoordinatorClient;
use crate::core::profile::DeploymentProfile;
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
    pub profile: DeploymentProfile,
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
    pub webhooks: Arc<crate::webhook::WebhookService>,
    pub webhook_worker: Arc<crate::webhook::WebhookOutboxWorker>,
    pub workflows: Arc<crate::workflow::WorkflowService>,
    pub scheduler: Arc<crate::workflow::WorkflowScheduler>,
    pub runners: Arc<crate::runner::RunnerService>,
    pub runner_executor: Arc<crate::runner::IsolatedRunnerExecutor>,
    pub search: Arc<crate::search::SearchService>,
    pub discussions: Arc<crate::discussions::DiscussionService>,
    pub discovery: Arc<crate::discovery::DiscoveryService>,
    pub security: Arc<crate::security::SecurityService>,
    pub public_origin: String,
    pub consistency_barrier: Arc<crate::backup::ConsistencyBarrier>,
    pub storage_nodes: Vec<(String, String)>,
    pub active_maintenance: Arc<tokio::sync::Mutex<Option<crate::backup::MaintenanceGuard>>>,
    pub upload_semaphore: Arc<tokio::sync::Semaphore>,
}

impl AppState {
    pub async fn acquire_upload_permit(
        &self,
    ) -> Result<tokio::sync::OwnedSemaphorePermit, crate::core::error::ForgeError> {
        self.upload_semaphore
            .clone()
            .try_acquire_owned()
            .map_err(|_| {
                crate::core::error::ForgeError::RateLimited(
                    "Upload concurrency limit reached. Please retry your upload shortly."
                        .to_string(),
                )
            })
    }

    pub fn storage_root(&self) -> &std::path::Path {
        self.vcs.storage_root()
    }

    pub fn with_public_origin(mut self, origin: impl Into<String>) -> Self {
        self.public_origin = origin.into();
        self
    }

    pub fn new_with_defaults(
        db: Arc<dyn Database>,
        storage_root: &std::path::Path,
        coordinator: Option<CoordinatorClient>,
    ) -> Result<Self, anyhow::Error> {
        Self::new_with_profile(
            DeploymentProfile::Development,
            db,
            storage_root,
            coordinator,
        )
    }

    pub fn new_with_profile(
        profile: DeploymentProfile,
        db: Arc<dyn Database>,
        storage_root: &std::path::Path,
        coordinator: Option<CoordinatorClient>,
    ) -> Result<Self, anyhow::Error> {
        let permissions = Arc::new(PermissionService::new(Arc::clone(&db)));
        let repositories = Arc::new(RepositoryService::new(
            Arc::clone(&db),
            Arc::clone(&permissions),
        ));
        let vcs = Arc::new(NativeVcsService::new(storage_root));
        let rate_limiter = Arc::new(RateLimiter::new());
        let mail = Arc::new(crate::auth::DevMailCatcher::new());

        let artifact_store_path = storage_root.join("artifacts");
        let artifact_store = Arc::new(spine_artifact::LocalArtifactStore::new(
            artifact_store_path,
        )?);
        let artifacts = Arc::new(ArtifactService::new(Arc::clone(&db), artifact_store));
        let releases = Arc::new(ReleaseService::new(Arc::clone(&db), Arc::clone(&artifacts)));
        let packages = Arc::new(PackageService::new(Arc::clone(&db), Arc::clone(&artifacts)));
        let cargo = Arc::new(CargoRegistryAdapter::new(Arc::clone(&packages)));

        let ssrf = crate::webhook::SsrfValidator::new(profile != DeploymentProfile::CoreForge);
        let webhooks = Arc::new(crate::webhook::WebhookService::new(
            Arc::clone(&db),
            ssrf.clone(),
        ));
        let webhook_worker = Arc::new(crate::webhook::WebhookOutboxWorker::new(
            Arc::clone(&db),
            ssrf,
        ));
        let scheduler = Arc::new(
            crate::workflow::WorkflowScheduler::new(Arc::clone(&db), Some((*webhooks).clone()))
                .with_profile(profile),
        );
        let workflows = Arc::new(
            crate::workflow::WorkflowService::new(Arc::clone(&db), Some((*webhooks).clone()))
                .with_profile(profile),
        );
        let runners = Arc::new(
            crate::runner::RunnerService::new(Arc::clone(&db), (*scheduler).clone())
                .with_profile(profile),
        );
        let runner_workspace_root = storage_root.join("runner_workspaces");
        let runner_executor = Arc::new(
            crate::runner::IsolatedRunnerExecutor::new(runner_workspace_root).with_profile(profile),
        );

        let search = Arc::new(crate::search::SearchService::new(
            Arc::clone(&db),
            Arc::clone(&permissions),
            storage_root.to_path_buf(),
        ));
        let discussions = Arc::new(crate::discussions::DiscussionService::new(
            Arc::clone(&db),
            Arc::clone(&permissions),
        ));
        let discovery = Arc::new(crate::discovery::DiscoveryService::new(
            Arc::clone(&db),
            Arc::clone(&permissions),
            storage_root.to_path_buf(),
        ));
        let security = Arc::new(crate::security::SecurityService::new(
            Arc::clone(&db),
            Arc::clone(&permissions),
            storage_root.to_path_buf(),
        ));

        let public_origin = std::env::var("FORGE_PUBLIC_ORIGIN")
            .or_else(|_| std::env::var("APP_URL"))
            .unwrap_or_else(|_| "http://127.0.0.1:3000".to_string());

        let consistency_barrier = Arc::new(
            crate::backup::ConsistencyBarrier::new().with_storage_root(vcs.storage_root()),
        );

        Ok(Self {
            profile,
            db,
            permissions,
            repositories,
            vcs,
            rate_limiter,
            mail,
            coordinator,
            artifacts,
            releases,
            packages,
            cargo,
            webhooks,
            webhook_worker,
            workflows,
            scheduler,
            runners,
            runner_executor,
            search,
            discussions,
            discovery,
            security,
            public_origin,
            consistency_barrier,
            storage_nodes: Vec::new(),
            active_maintenance: Arc::new(tokio::sync::Mutex::new(None)),
            upload_semaphore: Arc::new(tokio::sync::Semaphore::new(16)),
        })
    }
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
                trimmed
                    .strip_prefix("spine_session=")
                    .map(|tok| tok.trim().to_string())
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
