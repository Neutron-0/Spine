use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{header, StatusCode},
    response::Response,
    Extension, Json,
};
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct CreateNamespaceRequest {
    pub name: String,
    pub description: Option<String>,
    pub is_private: Option<bool>,
}

/// GET /api/v1/artifacts/quota
pub async fn get_quota(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
) -> Result<Json<ArtifactQuota>, ForgeError> {
    let current_user = user
        .ok_or_else(|| ForgeError::Unauthorized("Authentication required".to_string()))?
        .0;
    let quota = state
        .db
        .get_or_create_quota("user", &current_user.id)
        .await?;
    Ok(Json(quota))
}

/// POST /api/v1/artifacts/namespaces
pub async fn create_namespace(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(payload): Json<CreateNamespaceRequest>,
) -> Result<(StatusCode, Json<ArtifactNamespaceDto>), ForgeError> {
    let current_user = user
        .ok_or_else(|| ForgeError::Unauthorized("Authentication required".to_string()))?
        .0;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;

    let ns = ArtifactNamespace {
        id: uuid::Uuid::new_v4().to_string(),
        name: payload.name.trim().to_string(),
        owner_type: "user".to_string(),
        owner_id: current_user.id,
        is_private: payload.is_private.unwrap_or(false),
        description: payload.description.unwrap_or_default(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_artifact_namespace(ns).await?;
    Ok((
        StatusCode::CREATED,
        Json(ArtifactNamespaceDto::from(&created)),
    ))
}

/// GET /api/v1/artifacts/namespaces
pub async fn list_namespaces(
    State(state): State<AppState>,
) -> Result<Json<Vec<ArtifactNamespaceDto>>, ForgeError> {
    let list = state.db.list_artifact_namespaces().await?;
    let dtos = list.iter().map(ArtifactNamespaceDto::from).collect();
    Ok(Json(dtos))
}

/// POST /api/v1/artifacts/:namespace/:artifact/versions/:version
pub async fn upload_artifact_version(
    State(state): State<AppState>,
    Path((ns_name, artifact_name, version)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
    body: Bytes,
) -> Result<(StatusCode, Json<ArtifactVersionDto>), ForgeError> {
    let current_user = user
        .ok_or_else(|| ForgeError::Unauthorized("Authentication required".to_string()))?
        .0;

    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    if ns.owner_id != current_user.id {
        return Err(ForgeError::Forbidden(
            "Only the namespace owner can publish versions".to_string(),
        ));
    }

    if let Some(_) = state
        .db
        .get_artifact_version(&ns.id, &artifact_name, &version)
        .await?
    {
        return Err(ForgeError::Conflict(format!(
            "Version '{}' of artifact '{}' already exists",
            version, artifact_name
        )));
    }

    let stored = state
        .artifacts
        .store_object_bytes(
            &ns.owner_type,
            &ns.owner_id,
            &format!("{}-{}.bin", artifact_name, version),
            Some("application/octet-stream".to_string()),
            &body,
        )
        .await?;

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as i64;

    let ver = ArtifactVersion {
        id: uuid::Uuid::new_v4().to_string(),
        namespace_id: ns.id,
        artifact_name: artifact_name.clone(),
        version: version.clone(),
        manifest_id: stored.id.clone(),
        whole_hash: stored.whole_object_hash.clone(),
        size_bytes: body.len() as i64,
        created_by_user_id: current_user.id,
        created_at_nanos: now,
    };

    let created = state.db.create_artifact_version(ver).await?;
    Ok((
        StatusCode::CREATED,
        Json(ArtifactVersionDto {
            id: created.id,
            namespace_id: created.namespace_id,
            artifact_name: created.artifact_name,
            version: created.version,
            manifest_id: created.manifest_id,
            whole_hash: created.whole_hash,
            size_bytes: created.size_bytes,
            created_by_user_id: created.created_by_user_id,
            created_at_nanos: created.created_at_nanos,
        }),
    ))
}

/// GET /api/v1/artifacts/:namespace/:artifact/versions
pub async fn list_artifact_versions(
    State(state): State<AppState>,
    Path((ns_name, artifact_name)): Path<(String, String)>,
) -> Result<Json<Vec<ArtifactVersionDto>>, ForgeError> {
    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    let list = state
        .db
        .list_artifact_versions(&ns.id, &artifact_name)
        .await?;
    let dtos = list
        .into_iter()
        .map(|v| ArtifactVersionDto {
            id: v.id,
            namespace_id: v.namespace_id,
            artifact_name: v.artifact_name,
            version: v.version,
            manifest_id: v.manifest_id,
            whole_hash: v.whole_hash,
            size_bytes: v.size_bytes,
            created_by_user_id: v.created_by_user_id,
            created_at_nanos: v.created_at_nanos,
        })
        .collect();
    Ok(Json(dtos))
}

/// GET /api/v1/artifacts/:namespace/:artifact/versions/:version/download
pub async fn download_artifact_version(
    State(state): State<AppState>,
    Path((ns_name, artifact_name, version)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    let ver = state
        .db
        .get_artifact_version(&ns.id, &artifact_name, &version)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Artifact version not found".to_string()))?;

    let bytes = state.artifacts.reconstruct_object(&ver.manifest_id).await?;

    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}-{}.bin\"", artifact_name, version),
        )
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    Ok(res)
}
