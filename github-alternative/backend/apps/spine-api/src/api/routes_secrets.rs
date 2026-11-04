use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::{User, WorkflowSecret};

#[derive(Deserialize)]
pub struct UpsertSecretRequest {
    pub value: String,
}

#[derive(Serialize)]
pub struct SecretSummaryDto {
    pub id: String,
    pub name: String,
    pub owner_type: String,
    pub owner_id: String,
    pub created_at_nanos: i64,
    pub updated_at_nanos: i64,
}

pub async fn upsert_repo_secret(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, secret_name)): Path<(String, String, String)>,
    Json(payload): Json<UpsertSecretRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_admin_access(Some(&caller), &repo)
        .await?;

    let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let secret = WorkflowSecret {
        id: Uuid::new_v4().to_string(),
        owner_type: "REPO".to_string(),
        owner_id: repo.id.clone(),
        name: secret_name.to_uppercase(),
        ciphertext: payload.value,
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
    };

    let saved = state.db.upsert_workflow_secret(secret).await?;
    let summary = SecretSummaryDto {
        id: saved.id,
        name: saved.name,
        owner_type: saved.owner_type,
        owner_id: saved.owner_id,
        created_at_nanos: saved.created_at_nanos,
        updated_at_nanos: saved.updated_at_nanos,
    };

    Ok((StatusCode::OK, Json(summary)))
}

pub async fn list_repo_secrets(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_admin_access(Some(&caller), &repo)
        .await?;

    let secrets = state.db.list_workflow_secrets("REPO", &repo.id).await?;
    let summaries: Vec<SecretSummaryDto> = secrets
        .into_iter()
        .map(|s| SecretSummaryDto {
            id: s.id,
            name: s.name,
            owner_type: s.owner_type,
            owner_id: s.owner_id,
            created_at_nanos: s.created_at_nanos,
            updated_at_nanos: s.updated_at_nanos,
        })
        .collect();

    Ok(Json(summaries))
}

pub async fn delete_repo_secret(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, secret_name)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_admin_access(Some(&caller), &repo)
        .await?;

    state
        .db
        .delete_workflow_secret("REPO", &repo.id, &secret_name.to_uppercase())
        .await?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn upsert_org_secret(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, secret_name)): Path<(String, String)>,
    Json(payload): Json<UpsertSecretRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let secret = WorkflowSecret {
        id: Uuid::new_v4().to_string(),
        owner_type: "ORG".to_string(),
        owner_id: org.id.clone(),
        name: secret_name.to_uppercase(),
        ciphertext: payload.value,
        created_at_nanos: now_nanos,
        updated_at_nanos: now_nanos,
    };

    let saved = state.db.upsert_workflow_secret(secret).await?;
    let summary = SecretSummaryDto {
        id: saved.id,
        name: saved.name,
        owner_type: saved.owner_type,
        owner_id: saved.owner_id,
        created_at_nanos: saved.created_at_nanos,
        updated_at_nanos: saved.updated_at_nanos,
    };

    Ok((StatusCode::OK, Json(summary)))
}

pub async fn list_org_secrets(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let secrets = state.db.list_workflow_secrets("ORG", &org.id).await?;
    let summaries: Vec<SecretSummaryDto> = secrets
        .into_iter()
        .map(|s| SecretSummaryDto {
            id: s.id,
            name: s.name,
            owner_type: s.owner_type,
            owner_id: s.owner_id,
            created_at_nanos: s.created_at_nanos,
            updated_at_nanos: s.updated_at_nanos,
        })
        .collect();

    Ok(Json(summaries))
}

pub async fn delete_org_secret(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((org_name, secret_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    state
        .db
        .delete_workflow_secret("ORG", &org.id, &secret_name.to_uppercase())
        .await?;

    Ok(StatusCode::NO_CONTENT)
}
