use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use crate::packages::cargo::{CargoPublishResponse, CARGO_ECOSYSTEM};
use axum::{
    body::Bytes,
    extract::{Host, Path, State},
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};

/// GET /index/config.json
pub async fn cargo_index_config(
    Host(host): Host,
) -> impl IntoResponse {
    let scheme = if host.starts_with("localhost") || host.starts_with("127.0.0.1") {
        "http"
    } else {
        "https"
    };
    let base_url = format!("{}://{}", scheme, host);
    Json(crate::packages::CargoRegistryAdapter::config_json(&base_url))
}

/// GET /index/:prefix/:crate_name (1 or 2 letter crate, or non-nested)
pub async fn cargo_index_entry_single(
    State(state): State<AppState>,
    Path((_prefix, crate_name)): Path<(String, String)>,
) -> Result<Response, ForgeError> {
    serve_crate_index(&state, &crate_name).await
}

/// GET /index/:p1/:p2/:crate_name (3 letter or 4+ letter prefix)
pub async fn cargo_index_entry_double(
    State(state): State<AppState>,
    Path((_p1, _p2, crate_name)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    serve_crate_index(&state, &crate_name).await
}

async fn serve_crate_index(state: &AppState, crate_name: &str) -> Result<Response, ForgeError> {
    let index_file = state.cargo.get_index_file(crate_name).await?;
    match index_file {
        Some(content) => {
            let res = Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/plain")
                .header(header::CONTENT_LENGTH, content.len().to_string())
                .body(axum::body::Body::from(content))
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            Ok(res)
        }
        None => Err(ForgeError::NotFound(format!("Crate '{}' not found", crate_name))),
    }
}

/// PUT /api/v1/crates/new
pub async fn cargo_publish(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    body: Bytes,
) -> Result<Json<CargoPublishResponse>, ForgeError> {
    let current_user = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to publish crate".to_string()))?.0;
    let res = state.cargo.publish(&body, &current_user.id).await?;
    Ok(Json(res))
}

/// GET /api/v1/crates/:crate_name/:version/download
pub async fn cargo_download(
    State(state): State<AppState>,
    Path((crate_name, version)): Path<(String, String)>,
) -> Result<Response, ForgeError> {
    let bytes = state.cargo.download(&crate_name, &version).await?;
    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-tar")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}-{}.crate\"", crate_name, version),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    Ok(res)
}

/// DELETE /api/v1/crates/:crate_name/:version/yank
pub async fn cargo_yank(
    State(state): State<AppState>,
    Path((crate_name, version)): Path<(String, String)>,
    user: Option<Extension<User>>,
) -> Result<Json<serde_json::Value>, ForgeError> {
    let _current_user = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to yank crate".to_string()))?.0;
    state.cargo.set_yanked(&crate_name, &version, true).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// PUT /api/v1/crates/:crate_name/:version/unyank
pub async fn cargo_unyank(
    State(state): State<AppState>,
    Path((crate_name, version)): Path<(String, String)>,
    user: Option<Extension<User>>,
) -> Result<Json<serde_json::Value>, ForgeError> {
    let _current_user = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to unyank crate".to_string()))?.0;
    state.cargo.set_yanked(&crate_name, &version, false).await?;
    Ok(Json(serde_json::json!({ "ok": true })))
}

/// GET /api/v1/packages
pub async fn list_packages(
    State(state): State<AppState>,
) -> Result<Json<Vec<PackageDto>>, ForgeError> {
    let list = state.packages.list_packages(CARGO_ECOSYSTEM).await?;
    Ok(Json(list))
}

/// GET /api/v1/packages/:ecosystem/:name
pub async fn get_package(
    State(state): State<AppState>,
    Path((ecosystem, name)): Path<(String, String)>,
) -> Result<Json<PackageDto>, ForgeError> {
    let pkg = state
        .packages
        .get_package(&ecosystem, &name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Package '{}' not found", name)))?;
    Ok(Json(pkg))
}
