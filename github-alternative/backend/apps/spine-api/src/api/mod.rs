pub mod middleware;
pub mod routes_activity;
pub mod routes_artifacts;
pub mod routes_auth;
pub mod routes_health;
pub mod routes_issues;
pub mod routes_lfs;
pub mod routes_notifications;
pub mod routes_orgs;
pub mod routes_packages;
pub mod routes_protection;
pub mod routes_pulls;
pub mod routes_releases;
pub mod routes_repos;
pub mod routes_reviews;
pub mod routes_teams;
pub mod routes_users;

pub use middleware::AppState;

use axum::{
    middleware as axum_middleware,
    routing::{delete, get, patch, post, put},
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
        // Organization endpoints (Batch 09)
        .route(
            "/api/v1/orgs",
            get(routes_orgs::list_organizations).post(routes_orgs::create_organization),
        )
        .route(
            "/api/v1/orgs/:org_name",
            get(routes_orgs::get_organization)
                .patch(routes_orgs::update_organization)
                .delete(routes_orgs::delete_organization),
        )
        .route(
            "/api/v1/orgs/:org_name/members",
            get(routes_orgs::list_organization_members)
                .post(routes_orgs::add_or_update_member),
        )
        .route(
            "/api/v1/orgs/:org_name/members/:user_id",
            delete(routes_orgs::remove_member),
        )
        .route(
            "/api/v1/orgs/:org_name/invitations",
            get(routes_orgs::list_invitations)
                .post(routes_orgs::create_invitation),
        )
        .route(
            "/api/v1/orgs/invitations/:token/accept",
            post(routes_orgs::accept_invitation),
        )
        .route(
            "/api/v1/orgs/:org_name/repos",
            get(routes_orgs::list_org_repositories)
                .post(routes_orgs::create_org_repository),
        )
        // Team endpoints (Batch 09)
        .route(
            "/api/v1/orgs/:org_name/teams",
            get(routes_teams::list_teams).post(routes_teams::create_team),
        )
        .route(
            "/api/v1/orgs/:org_name/teams/:team_slug",
            get(routes_teams::get_team).delete(routes_teams::delete_team),
        )
        .route(
            "/api/v1/orgs/:org_name/teams/:team_slug/members",
            get(routes_teams::list_team_members)
                .post(routes_teams::add_team_member),
        )
        .route(
            "/api/v1/orgs/:org_name/teams/:team_slug/members/:user_id",
            delete(routes_teams::remove_team_member),
        )
        .route(
            "/api/v1/orgs/:org_name/teams/:team_slug/repos/:repo_name",
            put(routes_teams::set_team_repository_permission),
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
        .route(
            "/api/v1/repos/:owner/:name/capability",
            post(routes_repos::issue_capability_ticket),
        )
        .route(
            "/api/v1/repos/:owner/:name/status/durability",
            get(routes_repos::get_repo_durability_status),
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
        // Activity events endpoint (Batch 09)
        .route(
            "/api/v1/repos/:owner/:name/activity",
            get(routes_activity::list_repo_activity),
        )
        // Issues, Comments, Labels, Milestones (Batch 09)
        .route(
            "/api/v1/repos/:owner/:name/issues",
            get(routes_issues::list_issues).post(routes_issues::create_issue),
        )
        .route(
            "/api/v1/repos/:owner/:name/issues/:number",
            get(routes_issues::get_issue).patch(routes_issues::update_issue),
        )
        .route(
            "/api/v1/repos/:owner/:name/issues/:number/comments",
            get(routes_issues::list_issue_comments)
                .post(routes_issues::create_issue_comment),
        )
        .route(
            "/api/v1/repos/:owner/:name/labels",
            get(routes_issues::list_labels).post(routes_issues::create_label),
        )
        .route(
            "/api/v1/repos/:owner/:name/labels/:label_id",
            delete(routes_issues::delete_label),
        )
        .route(
            "/api/v1/repos/:owner/:name/milestones",
            get(routes_issues::list_milestones)
                .post(routes_issues::create_milestone),
        )
        .route(
            "/api/v1/repos/:owner/:name/milestones/:milestone_id",
            patch(routes_issues::update_milestone)
                .delete(routes_issues::delete_milestone),
        )
        // Pull Requests & Native Diffs & Merges (Batch 09)
        .route(
            "/api/v1/repos/:owner/:name/pulls",
            get(routes_pulls::list_pull_requests)
                .post(routes_pulls::create_pull_request),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number",
            get(routes_pulls::get_pull_request)
                .patch(routes_pulls::update_pull_request),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/diff",
            get(routes_pulls::get_pull_request_diff),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/mergeability",
            get(routes_pulls::get_pull_request_mergeability),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/merge",
            post(routes_pulls::execute_pull_request_merge),
        )
        // Reviews & Review Comments (Batch 09)
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/reviews",
            get(routes_reviews::list_reviews)
                .post(routes_reviews::create_review),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/reviews/:review_id/dismiss",
            post(routes_reviews::dismiss_review),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/reviewers",
            post(routes_reviews::request_reviewer),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/comments",
            get(routes_reviews::list_review_comments)
                .post(routes_reviews::create_review_comment),
        )
        .route(
            "/api/v1/repos/:owner/:name/pulls/:number/comments/:comment_id/resolve",
            patch(routes_reviews::resolve_review_comment),
        )
        // Branch Protection (Batch 09)
        .route(
            "/api/v1/repos/:owner/:name/protection",
            get(routes_protection::list_branch_protection)
                .post(routes_protection::set_branch_protection),
        )
        .route(
            "/api/v1/repos/:owner/:name/protection/:rule_id",
            delete(routes_protection::delete_branch_protection),
        )
        // Notifications (Batch 09)
        .route(
            "/api/v1/notifications",
            get(routes_notifications::list_notifications),
        )
        .route(
            "/api/v1/notifications/:notif_id/read",
            patch(routes_notifications::mark_notification_read),
        )
        .route(
            "/api/v1/notifications/read-all",
            post(routes_notifications::mark_all_notifications_read),
        )
        // Spine LFS (Batch 11)
        .route(
            "/api/v1/repos/:owner/:name/lfs/objects/batch",
            post(routes_lfs::lfs_batch),
        )
        .route(
            "/api/v1/repos/:owner/:name/lfs/sessions/:session_id/chunks/:index/:hash",
            put(routes_lfs::lfs_upload_chunk),
        )
        .route(
            "/api/v1/repos/:owner/:name/lfs/sessions/:session_id/finalize",
            post(routes_lfs::lfs_finalize),
        )
        .route(
            "/api/v1/repos/:owner/:name/lfs/objects/:manifest_id/download",
            get(routes_lfs::lfs_download_object),
        )
        .route(
            "/api/v1/repos/:owner/:name/lfs/chunks/:chunk_hash",
            get(routes_lfs::lfs_download_chunk),
        )
        // Releases & Assets (Batch 11)
        .route(
            "/api/v1/repos/:owner/:name/releases",
            get(routes_releases::list_releases).post(routes_releases::create_release),
        )
        .route(
            "/api/v1/repos/:owner/:name/releases/:release_id",
            get(routes_releases::get_release)
                .patch(routes_releases::update_release)
                .delete(routes_releases::delete_release),
        )
        .route(
            "/api/v1/repos/:owner/:name/releases/tags/:tag",
            get(routes_releases::get_release_by_tag),
        )
        .route(
            "/api/v1/repos/:owner/:name/releases/:release_id/assets",
            post(routes_releases::upload_release_asset),
        )
        .route(
            "/api/v1/repos/:owner/:name/releases/:release_id/assets/:asset_id/download",
            get(routes_releases::download_release_asset),
        )
        .route(
            "/api/v1/repos/:owner/:name/releases/:release_id/assets/:asset_id",
            delete(routes_releases::delete_release_asset),
        )
        // Generic Artifacts & Quotas (Batch 11)
        .route("/api/v1/artifacts/quota", get(routes_artifacts::get_quota))
        .route(
            "/api/v1/artifacts/namespaces",
            get(routes_artifacts::list_namespaces).post(routes_artifacts::create_namespace),
        )
        .route(
            "/api/v1/artifacts/:namespace/:artifact/versions",
            get(routes_artifacts::list_artifact_versions),
        )
        .route(
            "/api/v1/artifacts/:namespace/:artifact/versions/:version",
            post(routes_artifacts::upload_artifact_version),
        )
        .route(
            "/api/v1/artifacts/:namespace/:artifact/versions/:version/download",
            get(routes_artifacts::download_artifact_version),
        )
        // Package Registry & Cargo (Batch 11)
        .route("/index/config.json", get(routes_packages::cargo_index_config))
        .route("/index/:p1/:crate_name", get(routes_packages::cargo_index_entry_single))
        .route("/index/:p1/:p2/:crate_name", get(routes_packages::cargo_index_entry_double))
        .route("/api/v1/crates/new", put(routes_packages::cargo_publish))
        .route("/api/v1/crates/:crate_name/:version/download", get(routes_packages::cargo_download))
        .route("/api/v1/crates/:crate_name/:version/yank", delete(routes_packages::cargo_yank))
        .route("/api/v1/crates/:crate_name/:version/unyank", put(routes_packages::cargo_unyank))
        .route("/api/v1/packages", get(routes_packages::list_packages))
        .route("/api/v1/packages/:ecosystem/:name", get(routes_packages::get_package))
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
