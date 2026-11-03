use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct NotificationQuery {
    pub unread_only: Option<bool>,
}

pub async fn list_notifications(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Query(query): Query<NotificationQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let unread_only = query.unread_only.unwrap_or(false);

    let notifs = state.db.list_user_notifications(&caller.id, unread_only).await?;
    let dtos: Vec<NotificationDto> = notifs.iter().map(NotificationDto::from).collect();
    Ok(Json(dtos))
}

pub async fn mark_notification_read(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path(notif_id): Path<String>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    state.db.mark_notification_read(&notif_id, &caller.id).await?;
    Ok(StatusCode::OK)
}

pub async fn mark_all_notifications_read(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    state.db.mark_all_notifications_read(&caller.id).await?;
    Ok(StatusCode::OK)
}
