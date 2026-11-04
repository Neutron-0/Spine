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
use serde::{Deserialize, Serialize};
use spine_artifact::ArtifactManifest;
use spine_cluster::durability::DurabilityPolicy;
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsBatchRequest {
    pub operation: String, // "upload" | "download"
    #[serde(default)]
    pub transfers: Vec<String>,
    pub objects: Vec<LfsObjectRequest>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectRequest {
    pub oid: String,
    pub size: u64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsBatchResponse {
    pub transfer: String,
    pub objects: Vec<LfsObjectResponse>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectResponse {
    pub oid: String,
    pub size: u64,
    pub authenticated: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actions: Option<HashMap<String, LfsAction>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<LfsObjectError>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsAction {
    pub href: String,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub header: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct LfsObjectError {
    pub code: u16,
    pub message: String,
}

/// POST /api/v1/repos/:owner/:repo/lfs/objects/batch
pub async fn lfs_batch(
    State(state): State<AppState>,
    Path((owner, repo_name)): Path<(String, String)>,
    user: Option<Extension<User>>,
    Json(payload): Json<LfsBatchRequest>,
) -> Result<Json<LfsBatchResponse>, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    // Check permissions
    if payload.operation == "upload" {
        state.permissions.check_write_access(caller, &repo).await?;
    } else {
        state.permissions.check_read_access(caller, &repo).await?;
    }

    let mut response_objects = Vec::new();

    if payload.operation == "upload" {
        let owner_type = if repo.owner_org_id.is_some() {
            "organization"
        } else {
            "user"
        };
        let owner_id = repo.owner_org_id.as_deref().unwrap_or(&repo.owner_user_id);

        for obj in payload.objects {
            // Check if manifest already exists
            if let Ok(Some(_)) = state.db.get_artifact_manifest(&obj.oid).await {
                // Object exists already, no actions needed
                response_objects.push(LfsObjectResponse {
                    oid: obj.oid,
                    size: obj.size,
                    authenticated: true,
                    actions: None,
                    error: None,
                });
                continue;
            }

            // Create upload session
            match state
                .artifacts
                .create_upload_session(
                    owner_type,
                    owner_id,
                    &obj.oid,
                    obj.size,
                    spine_artifact::DEFAULT_CHUNK_SIZE,
                    DurabilityPolicy::SingleCopy,
                    86400, // 24 hours
                )
                .await
            {
                Ok(session) => {
                    let mut actions = HashMap::new();
                    actions.insert(
                        "upload".to_string(),
                        LfsAction {
                            href: format!(
                                "/api/v1/repos/{}/{}/lfs/sessions/{}/chunks",
                                owner, repo_name, session.id
                            ),
                            header: HashMap::new(),
                            expires_in: Some(86400),
                        },
                    );
                    actions.insert(
                        "finalize".to_string(),
                        LfsAction {
                            href: format!(
                                "/api/v1/repos/{}/{}/lfs/sessions/{}/finalize",
                                owner, repo_name, session.id
                            ),
                            header: HashMap::new(),
                            expires_in: Some(86400),
                        },
                    );

                    response_objects.push(LfsObjectResponse {
                        oid: obj.oid,
                        size: obj.size,
                        authenticated: true,
                        actions: Some(actions),
                        error: None,
                    });
                }
                Err(e) => {
                    response_objects.push(LfsObjectResponse {
                        oid: obj.oid,
                        size: obj.size,
                        authenticated: true,
                        actions: None,
                        error: Some(LfsObjectError {
                            code: 422,
                            message: e.to_string(),
                        }),
                    });
                }
            }
        }
    } else {
        // Download operation
        for obj in payload.objects {
            if let Ok(Some(_)) = state.db.get_artifact_manifest(&obj.oid).await {
                let mut actions = HashMap::new();
                actions.insert(
                    "download".to_string(),
                    LfsAction {
                        href: format!(
                            "/api/v1/repos/{}/{}/lfs/objects/{}/download",
                            owner, repo_name, obj.oid
                        ),
                        header: HashMap::new(),
                        expires_in: Some(3600),
                    },
                );

                response_objects.push(LfsObjectResponse {
                    oid: obj.oid,
                    size: obj.size,
                    authenticated: true,
                    actions: Some(actions),
                    error: None,
                });
            } else {
                response_objects.push(LfsObjectResponse {
                    oid: obj.oid,
                    size: obj.size,
                    authenticated: true,
                    actions: None,
                    error: Some(LfsObjectError {
                        code: 404,
                        message: "Object not found".to_string(),
                    }),
                });
            }
        }
    }

    Ok(Json(LfsBatchResponse {
        transfer: "basic".to_string(),
        objects: response_objects,
    }))
}

/// PUT /api/v1/repos/:owner/:repo/lfs/sessions/:session_id/chunks/:index/:hash
pub async fn lfs_upload_chunk(
    State(state): State<AppState>,
    Path((_owner, _repo, session_id, index, hash)): Path<(String, String, String, u32, String)>,
    body: Bytes,
) -> Result<StatusCode, ForgeError> {
    state
        .artifacts
        .upload_chunk(&session_id, index, &hash, &body)
        .await?;
    Ok(StatusCode::OK)
}

/// POST /api/v1/repos/:owner/:repo/lfs/sessions/:session_id/finalize
pub async fn lfs_finalize(
    State(state): State<AppState>,
    Path((_owner, _repo, session_id)): Path<(String, String, String)>,
    Json(manifest): Json<ArtifactManifest>,
) -> Result<Json<StoredArtifactManifest>, ForgeError> {
    let stored = state
        .artifacts
        .finalize_upload(&session_id, manifest)
        .await?;
    Ok(Json(stored))
}

/// GET /api/v1/repos/:owner/:repo/lfs/objects/:manifest_id/download
pub async fn lfs_download_object(
    State(state): State<AppState>,
    Path((_owner, _repo, manifest_id)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let bytes = state.artifacts.reconstruct_object(&manifest_id).await?;
    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    Ok(res)
}

/// GET /api/v1/repos/:owner/:repo/lfs/chunks/:chunk_hash
pub async fn lfs_download_chunk(
    State(state): State<AppState>,
    Path((_owner, _repo, chunk_hash)): Path<(String, String, String)>,
) -> Result<Response, ForgeError> {
    let bytes = state.artifacts.get_chunk(&chunk_hash).await?;
    let res = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(header::CONTENT_LENGTH, bytes.len().to_string())
        .body(axum::body::Body::from(bytes))
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    Ok(res)
}
