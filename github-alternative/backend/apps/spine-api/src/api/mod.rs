pub mod middleware;
pub mod routes_auth;
pub mod routes_health;
pub mod routes_repos;
pub mod routes_users;

pub use middleware::AppState;

use axum::{
    middleware as axum_middleware,
    routing::{delete, get, post},
    Router,
};
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;

pub fn build_router(state: AppState) -> Router {
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    Router::new()
        // Health endpoints
        .route("/health", get(routes_health::health_liveness))
        .route("/health/live", get(routes_health::health_liveness))
        .route("/health/ready", get(routes_health::health_readiness))
        // Auth endpoints
        .route("/api/v1/auth/register", post(routes_auth::register))
        .route("/api/v1/auth/login", post(routes_auth::login))
        .route("/api/v1/auth/logout", post(routes_auth::logout))
        .route("/api/v1/auth/session", get(routes_auth::get_session))
        .route(
            "/api/v1/auth/password/change",
            post(routes_auth::change_password),
        )
        .route(
            "/api/v1/auth/password/reset/request",
            post(routes_auth::request_password_reset),
        )
        .route(
            "/api/v1/auth/password/reset/complete",
            post(routes_auth::complete_password_reset),
        )
        .route("/api/v1/auth/email/verify", post(routes_auth::verify_email))
        // User endpoints
        .route(
            "/api/v1/users/me",
            get(routes_users::get_me).patch(routes_users::get_me),
        )
        .route(
            "/api/v1/users/me/sessions",
            get(routes_users::list_sessions),
        )
        .route(
            "/api/v1/users/me/sessions/:session_id",
            delete(routes_users::revoke_session),
        )
        .route(
            "/api/v1/users/me/pats",
            get(routes_users::list_pats).post(routes_users::create_pat),
        )
        .route(
            "/api/v1/users/tokens",
            get(routes_users::list_pats).post(routes_users::create_pat),
        )
        .route(
            "/api/v1/users/me/pats/:pat_id",
            delete(routes_users::revoke_pat),
        )
        // Repository management endpoints
        .route(
            "/api/v1/repos",
            get(routes_repos::list_repositories).post(routes_repos::create_repository),
        )
        .route(
            "/api/v1/repos/:owner/:name",
            get(routes_repos::get_repository).delete(routes_repos::delete_repository),
        )
        // Repository inspection endpoints
        .route(
            "/api/v1/repos/:owner/:name/branches",
            get(routes_repos::list_repo_branches),
        )
        .route(
            "/api/v1/repos/:owner/:name/tags",
            get(routes_repos::list_repo_tags),
        )
        .route(
            "/api/v1/repos/:owner/:name/commits",
            get(routes_repos::list_repo_commits),
        )
        .route(
            "/api/v1/repos/:owner/:name/tree/:ref",
            get(routes_repos::get_repo_tree),
        )
        .route(
            "/api/v1/repos/:owner/:name/blob/:ref/*path",
            get(routes_repos::get_repo_blob),
        )
        // Layers
        .layer(axum_middleware::from_fn_with_state(
            state.clone(),
            middleware::auth_middleware,
        ))
        .layer(axum_middleware::from_fn(middleware::request_id_middleware))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}
