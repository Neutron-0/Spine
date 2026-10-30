use axum::{extract::Path, http::StatusCode, response::IntoResponse, routing::get, Json, Router};
use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use std::net::SocketAddr;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Clone)]
struct AppState {
    pub _api_version: &'static str,
}

#[derive(Serialize)]
struct HealthResponse {
    status: &'static str,
    version: &'static str,
    service: &'static str,
}

#[derive(Serialize, Deserialize, Clone)]
struct RepositoryMeta {
    id: String,
    owner: String,
    name: String,
    description: String,
    is_private: bool,
    default_branch: String,
    created_at: String,
}

#[derive(Deserialize)]
struct CreateRepoRequest {
    name: String,
    description: Option<String>,
    is_private: Option<bool>,
}

#[derive(Serialize)]
struct CommitDto {
    id: String,
    author_name: String,
    author_email: String,
    timestamp: i64,
    message: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "spine_api=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let state = Arc::new(AppState {
        _api_version: "0.1.0",
    });

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    let app = Router::new()
        .route("/health", get(health_check))
        .route(
            "/api/v1/repos",
            get(list_repositories).post(create_repository),
        )
        .route("/api/v1/repos/:owner/:repo/commits", get(get_repo_commits))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = SocketAddr::from(([127, 0, 0, 1], 3000));
    tracing::info!("Spine Forge API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

async fn health_check() -> impl IntoResponse {
    Json(HealthResponse {
        status: "ok",
        version: "0.1.0",
        service: "spine-forge-api",
    })
}

async fn list_repositories() -> impl IntoResponse {
    let mock_repos = vec![
        RepositoryMeta {
            id: "repo_01j9".into(),
            owner: "spine-org".into(),
            name: "spine-core".into(),
            description: "High-performance Rust-native distributed version-control engine".into(),
            is_private: false,
            default_branch: "main".into(),
            created_at: "2026-10-31T00:00:00Z".into(),
        },
        RepositoryMeta {
            id: "repo_02k1".into(),
            owner: "spine-org".into(),
            name: "spine-forge".into(),
            description: "Modern developer collaboration platform and distributed storage".into(),
            is_private: false,
            default_branch: "main".into(),
            created_at: "2026-10-31T00:00:00Z".into(),
        },
    ];
    Json(mock_repos)
}

async fn create_repository(
    Json(payload): Json<CreateRepoRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    if payload.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            "Repository name is required".into(),
        ));
    }

    let new_repo = RepositoryMeta {
        id: format!("repo_{}", &ObjectId::ZERO.to_hex()[..8]),
        owner: "current-user".into(),
        name: payload.name,
        description: payload.description.unwrap_or_default(),
        is_private: payload.is_private.unwrap_or(false),
        default_branch: "main".into(),
        created_at: "2026-10-31T00:15:00Z".into(),
    };

    Ok((StatusCode::CREATED, Json(new_repo)))
}

async fn get_repo_commits(Path((_owner, _repo)): Path<(String, String)>) -> impl IntoResponse {
    let mock_commits = vec![
        CommitDto {
            id: "e4d909c290d0fb1ca068ffaddf22cbd0add22cbd0add22cbd0add22cbd0add22".into(),
            author_name: "Alice Engineer".into(),
            author_email: "alice@spine.org".into(),
            timestamp: 1730332800000000000,
            message: "feat: implement initial content-addressed storage engine".into(),
        },
        CommitDto {
            id: "1c2b3a4f5e6d7c8b9a0f1e2d3c4b5a6f7e8d9c0b1a2f3e4d5c6b7a8f9e0d1c2b".into(),
            author_name: "Bob Architect".into(),
            author_email: "bob@spine.org".into(),
            timestamp: 1730332805000000000,
            message: "docs: add repository binary format specification".into(),
        },
    ];
    Json(mock_commits)
}
