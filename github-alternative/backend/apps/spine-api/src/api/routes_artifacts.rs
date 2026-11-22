use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
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
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
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
    user: Option<Extension<User>>,
) -> Result<Json<Vec<ArtifactNamespaceDto>>, ForgeError> {
    let caller_id = user.as_ref().map(|Extension(u)| u.id.as_str());
    let list = state.db.list_artifact_namespaces().await?;
    let dtos = list
        .into_iter()
        .filter(|ns| !ns.is_private || caller_id == Some(&ns.owner_id))
        .map(|ns| ArtifactNamespaceDto::from(&ns))
        .collect();
    Ok(Json(dtos))
}

/// POST /api/v1/artifacts/:namespace/:artifact/versions/:version
pub async fn upload_artifact_version(
    State(state): State<AppState>,
    Path((ns_name, artifact_name, version)): Path<(String, String, String)>,
    user: Option<Extension<User>>,
    body: axum::body::Body,
) -> Result<(StatusCode, Json<ArtifactVersionDto>), ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;

    // 1. Authentication check: must happen BEFORE reading body
    let current_user = user
        .ok_or_else(|| ForgeError::Unauthorized("Authentication required".to_string()))?
        .0;

    // 2. Validate artifact_name and version format: must happen BEFORE reading body
    let trimmed_artifact = artifact_name.trim();
    let trimmed_version = version.trim();
    if trimmed_artifact.is_empty()
        || trimmed_artifact.len() > 128
        || trimmed_version.is_empty()
        || trimmed_version.len() > 64
    {
        return Err(ForgeError::BadRequest(
            "Artifact name or version invalid length".into(),
        ));
    }
    if trimmed_artifact.contains('/')
        || trimmed_artifact.contains('\\')
        || trimmed_artifact.contains("..")
        || trimmed_artifact
            .chars()
            .any(|c| c.is_control() || c == '\r' || c == '\n' || c == '"')
    {
        return Err(ForgeError::BadRequest(
            "Artifact name contains illegal path traversal or control characters".into(),
        ));
    }
    if trimmed_version.contains('/')
        || trimmed_version.contains('\\')
        || trimmed_version.contains("..")
        || trimmed_version
            .chars()
            .any(|c| c.is_control() || c == '\r' || c == '\n' || c == '"')
    {
        return Err(ForgeError::BadRequest(
            "Version contains illegal path traversal or control characters".into(),
        ));
    }

    // 3. Namespace lookup: BEFORE reading body
    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    // 4. Namespace ownership check: BEFORE reading body
    if ns.owner_id != current_user.id {
        return Err(ForgeError::Forbidden(
            "Only the namespace owner can publish versions".to_string(),
        ));
    }

    // 5. Version conflict check: BEFORE reading body
    if state
        .db
        .get_artifact_version(&ns.id, &artifact_name, &version)
        .await?
        .is_some()
    {
        return Err(ForgeError::Conflict(format!(
            "Version '{}' of artifact '{}' already exists",
            version, artifact_name
        )));
    }

    // 6. Concurrency control: acquire an upload permit
    let _upload_slot = state.acquire_upload_permit().await?;

    // 7. Bounded streaming: stream body to tempfile on disk (peak RAM < 64 KiB)
    let (tempfile, total_bytes, _digest) = crate::api::streaming::stream_body_to_tempfile(
        body,
        crate::api::streaming::DEFAULT_MAX_UPLOAD_BYTES,
        crate::api::streaming::DEFAULT_STREAM_TIMEOUT,
    )
    .await?;

    // 8. Ingest artifact via reader into deduplicated chunk store
    let stored = state
        .artifacts
        .store_object_reader(
            &ns.owner_type,
            &ns.owner_id,
            &format!("{}-{}.bin", artifact_name, version),
            Some("application/octet-stream".to_string()),
            tempfile.into_file(),
            Some(total_bytes),
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
        size_bytes: stored.total_size,
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
    user: Option<Extension<User>>,
    Path((ns_name, artifact_name)): Path<(String, String)>,
) -> Result<Json<Vec<ArtifactVersionDto>>, ForgeError> {
    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    if ns.is_private {
        let current_user = user
            .ok_or_else(|| {
                ForgeError::Unauthorized(
                    "Authentication required for private namespace".to_string(),
                )
            })?
            .0;
        if ns.owner_id != current_user.id {
            return Err(ForgeError::Forbidden(
                "Access denied to private namespace".to_string(),
            ));
        }
    }

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
    user: Option<Extension<User>>,
    Path((ns_name, artifact_name, version)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let ns = state
        .db
        .get_artifact_namespace_by_name(&ns_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound(format!("Namespace '{}' not found", ns_name)))?;

    if ns.is_private {
        let current_user = user
            .ok_or_else(|| {
                ForgeError::Unauthorized(
                    "Authentication required for private namespace".to_string(),
                )
            })?
            .0;
        if ns.owner_id != current_user.id {
            return Err(ForgeError::Forbidden(
                "Access denied to private namespace".to_string(),
            ));
        }
    }

    let ver = state
        .db
        .get_artifact_version(&ns.id, &artifact_name, &version)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Artifact version not found".to_string()))?;

    let bytes = state.artifacts.reconstruct_object(&ver.manifest_id).await?;

    let filename = format!("{}-{}.bin", artifact_name, version);
    let cd_value = crate::api::streaming::format_content_disposition("attachment", &filename);

    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .header(header::CONTENT_DISPOSITION, cd_value)
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    Ok(res)
}
