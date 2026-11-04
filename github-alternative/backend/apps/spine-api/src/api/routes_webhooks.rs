use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::{CreateWebhookSubscriptionRequest, User, WebhookSubscriptionDto};

#[derive(Deserialize)]
pub struct CreateWebhookPayload {
    pub target_url: String,
    pub secret: Option<String>,
    pub events: Vec<String>,
    pub is_active: Option<bool>,
}

#[derive(Deserialize)]
pub struct DeliveryQuery {
    pub limit: Option<i64>,
}

pub async fn create_repo_webhook(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateWebhookPayload>,
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

    let req = CreateWebhookSubscriptionRequest {
        owner_type: "REPO".to_string(),
        owner_id: repo.id.clone(),
        target_url: payload.target_url,
        secret: payload.secret,
        events: payload.events,
        is_active: payload.is_active,
    };

    let sub = state
        .webhooks
        .create_subscription(req)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::Ssrf(s) => ForgeError::BadRequest(s.to_string()),
            crate::webhook::WebhookError::Validation(v) => ForgeError::UnprocessableEntity(v),
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    let dto = WebhookSubscriptionDto {
        id: sub.id,
        owner_type: sub.owner_type,
        owner_id: sub.owner_id,
        target_url: sub.target_url,
        events: sub
            .events
            .split(',')
            .map(|s| s.trim().to_string())
            .collect(),
        is_active: sub.is_active,
        created_at_nanos: sub.created_at_nanos,
        updated_at_nanos: sub.updated_at_nanos,
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

pub async fn list_repo_webhooks(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;
    if let Some(c) = caller {
        state.permissions.check_admin_access(Some(c), &repo).await?;
    } else {
        return Err(ForgeError::Unauthorized(
            "Authentication required to view webhooks".into(),
        ));
    }

    let subs = state
        .webhooks
        .list_subscriptions("REPO", &repo.id)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    let dtos: Vec<WebhookSubscriptionDto> = subs
        .into_iter()
        .map(|s| WebhookSubscriptionDto {
            id: s.id,
            owner_type: s.owner_type,
            owner_id: s.owner_id,
            target_url: s.target_url,
            events: s.events.split(',').map(|e| e.trim().to_string()).collect(),
            is_active: s.is_active,
            created_at_nanos: s.created_at_nanos,
            updated_at_nanos: s.updated_at_nanos,
        })
        .collect();

    Ok(Json(dtos))
}

pub async fn get_webhook(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let sub = state
        .webhooks
        .get_subscription(&id)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::NotFound => {
                ForgeError::NotFound("Webhook not found".into())
            }
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    let dto = WebhookSubscriptionDto {
        id: sub.id,
        owner_type: sub.owner_type,
        owner_id: sub.owner_id,
        target_url: sub.target_url,
        events: sub
            .events
            .split(',')
            .map(|e| e.trim().to_string())
            .collect(),
        is_active: sub.is_active,
        created_at_nanos: sub.created_at_nanos,
        updated_at_nanos: sub.updated_at_nanos,
    };

    Ok(Json(dto))
}

pub async fn delete_webhook(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let _caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    state
        .webhooks
        .delete_subscription(&id)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_webhook_deliveries(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(id): Path<String>,
    Query(query): Query<DeliveryQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let _caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let limit = query.limit.unwrap_or(50);
    let deliveries = state.db.list_webhook_deliveries(&id, limit).await?;
    Ok(Json(deliveries))
}

pub async fn create_org_webhook(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(org_name): Path<String>,
    Json(payload): Json<CreateWebhookPayload>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let org = state
        .db
        .get_organization_by_slug(&org_name)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Organization not found".into()))?;

    state.permissions.check_org_admin(&caller, &org.id).await?;

    let req = CreateWebhookSubscriptionRequest {
        owner_type: "ORG".to_string(),
        owner_id: org.id.clone(),
        target_url: payload.target_url,
        secret: payload.secret,
        events: payload.events,
        is_active: payload.is_active,
    };

    let sub = state
        .webhooks
        .create_subscription(req)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::Ssrf(s) => ForgeError::BadRequest(s.to_string()),
            crate::webhook::WebhookError::Validation(v) => ForgeError::UnprocessableEntity(v),
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    let dto = WebhookSubscriptionDto {
        id: sub.id,
        owner_type: sub.owner_type,
        owner_id: sub.owner_id,
        target_url: sub.target_url,
        events: sub
            .events
            .split(',')
            .map(|s| s.trim().to_string())
            .collect(),
        is_active: sub.is_active,
        created_at_nanos: sub.created_at_nanos,
        updated_at_nanos: sub.updated_at_nanos,
    };

    Ok((StatusCode::CREATED, Json(dto)))
}

pub async fn list_org_webhooks(
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

    let subs = state
        .webhooks
        .list_subscriptions("ORG", &org.id)
        .await
        .map_err(|e| match e {
            crate::webhook::WebhookError::Database(d) => d,
            _ => ForgeError::Internal(e.to_string()),
        })?;

    let dtos: Vec<WebhookSubscriptionDto> = subs
        .into_iter()
        .map(|s| WebhookSubscriptionDto {
            id: s.id,
            owner_type: s.owner_type,
            owner_id: s.owner_id,
            target_url: s.target_url,
            events: s.events.split(',').map(|e| e.trim().to_string()).collect(),
            is_active: s.is_active,
            created_at_nanos: s.created_at_nanos,
            updated_at_nanos: s.updated_at_nanos,
        })
        .collect();

    Ok(Json(dtos))
}
