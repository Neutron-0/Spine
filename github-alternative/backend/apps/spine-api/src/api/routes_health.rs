use super::middleware::AppState;
use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    Json,
};
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
    let coord_ok = match &state.coordinator {
        Some(c) => c.check_health().await,
        None => true, // Standalone mode without external coordinator
    };

    let overall = db_ok && coord_ok;
    let status_code = if overall {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };

    let resp = ReadinessResponse {
        status: if overall { "ready" } else { "not_ready" },
        database: db_ok,
        coordinator: coord_ok,
    };

    (status_code, Json(resp))
}
