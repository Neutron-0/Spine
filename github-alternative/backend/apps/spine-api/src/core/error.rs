use axum::{
    http::StatusCode,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ForgeError {
    #[error("Bad request: {0}")]
    BadRequest(String),

    #[error("Unauthorized: {0}")]
    Unauthorized(String),

    #[error("Forbidden: {0}")]
    Forbidden(String),

    #[error("Not found: {0}")]
    NotFound(String),

    #[error("Conflict: {0}")]
    Conflict(String),

    #[error("Unprocessable entity: {0}")]
    UnprocessableEntity(String),

    #[error("Rate limited: {0}")]
    RateLimited(String),

    #[error("Service unavailable: {0}")]
    ServiceUnavailable(String),

    #[error("Internal server error: {0}")]
    Internal(String),
}

#[derive(Serialize)]
struct ErrorEnvelope {
    error: ErrorDetail,
}

#[derive(Serialize)]
struct ErrorDetail {
    code: &'static str,
    message: String,
}

impl IntoResponse for ForgeError {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            ForgeError::BadRequest(_) => (StatusCode::BAD_REQUEST, "BAD_REQUEST"),
            ForgeError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "UNAUTHORIZED"),
            ForgeError::Forbidden(_) => (StatusCode::FORBIDDEN, "FORBIDDEN"),
            ForgeError::NotFound(_) => (StatusCode::NOT_FOUND, "NOT_FOUND"),
            ForgeError::Conflict(_) => (StatusCode::CONFLICT, "CONFLICT"),
            ForgeError::UnprocessableEntity(_) => {
                (StatusCode::UNPROCESSABLE_ENTITY, "UNPROCESSABLE_ENTITY")
            }
            ForgeError::RateLimited(_) => (StatusCode::TOO_MANY_REQUESTS, "TOO_MANY_REQUESTS"),
            ForgeError::ServiceUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "SERVICE_UNAVAILABLE")
            }
            ForgeError::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_SERVER_ERROR"),
        };

        // Ensure internal messages don't leak database credentials or private stack traces in production
        let message = match &self {
            ForgeError::Internal(_) => "An internal server error occurred".to_string(),
            _ => self.to_string(),
        };

        let body = Json(ErrorEnvelope {
            error: ErrorDetail { code, message },
        });

        (status, body).into_response()
    }
}
