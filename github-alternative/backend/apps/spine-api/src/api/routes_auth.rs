use super::middleware::AppState;
use crate::auth::mail::{generate_reset_token, generate_verification_token, hash_token};
use crate::auth::password::{hash_password, validate_password_strength, verify_password};
use crate::auth::session::{generate_session_token, hash_session_token, DEFAULT_SESSION_TTL_NANOS};
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct AuthSuccessResponse {
    pub user: UserDto,
    pub token: String,
    pub expires_at_nanos: i64,
}

#[derive(Deserialize)]
pub struct LoginRequest {
    pub login: String, // username or email
    pub password: String,
}

#[derive(Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordRequest {
    pub email: String,
}

#[derive(Deserialize)]
pub struct ResetPasswordCompleteRequest {
    pub token: String,
    pub new_password: String,
}

#[derive(Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct GenericMessageResponse {
    pub message: &'static str,
}

pub async fn register(
    State(state): State<AppState>,
    Json(payload): Json<RegisterRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let username = payload.username.trim();
    if username.len() < 3 || username.len() > 32 {
        return Err(ForgeError::BadRequest(
            "Username must be between 3 and 32 characters".into(),
        ));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(ForgeError::BadRequest(
            "Username may only contain letters, numbers, hyphens, and underscores".into(),
        ));
    }

    let email = payload.email.trim();
    if !email.contains('@') || !email.contains('.') {
        return Err(ForgeError::BadRequest("Invalid email address".into()));
    }

    validate_password_strength(&payload.password)?;

    let pwd_hash = hash_password(&payload.password)?;
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let user_id = format!("usr_{}", uuid::Uuid::new_v4().simple());
    let display = payload.display_name.unwrap_or_else(|| username.to_string());

    let user = User {
        id: user_id.clone(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: email.to_string(),
        normalized_email: email.to_lowercase(),
        display_name: display,
        password_hash: pwd_hash,
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: now,
        updated_at_nanos: now,
        last_login_nanos: Some(now),
    };

    let created_user = state.db.create_user(user).await?;

    // Create session
    let token = generate_session_token();
    let token_hash = hash_session_token(&token);
    let expires_at = now + DEFAULT_SESSION_TTL_NANOS;

    let session = UserSession {
        id: format!("sess_{}", uuid::Uuid::new_v4().simple()),
        user_id: created_user.id.clone(),
        session_token_hash: token_hash,
        ip_address: None,
        user_agent: None,
        created_at_nanos: now,
        expires_at_nanos: expires_at,
        revoked: false,
    };
    state.db.create_session(session).await?;

    // Send verification token in background
    let v_token = generate_verification_token();
    let v_hash = hash_token(&v_token);
    let _ = state
        .db
        .create_email_token(&v_hash, &created_user.id, now + 24 * 3600 * 1_000_000_000)
        .await;
    let _ = state
        .mail
        .send_email(
            &created_user.email,
            "Verify your Spine Forge account",
            &format!("Your verification token is: {}", v_token),
        )
        .await;

    // Cookie
    let secure_flag = if state.public_origin.starts_with("https://")
        || state.profile == crate::core::profile::DeploymentProfile::CoreForge
    {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "spine_session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800{}",
        token, secure_flag
    );
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());

    let resp = AuthSuccessResponse {
        user: UserDto::from(&created_user),
        token,
        expires_at_nanos: expires_at,
    };

    Ok((StatusCode::CREATED, headers, Json(resp)))
}

pub async fn login(
    State(state): State<AppState>,
    Json(payload): Json<LoginRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let login_str = payload.login.trim();
    if login_str.is_empty() || payload.password.is_empty() {
        return Err(ForgeError::BadRequest("Credentials required".into()));
    }

    // Rate limiting
    if !state
        .rate_limiter
        .check_and_record(&format!("login_{}", login_str.to_lowercase()), 5, 60)
    {
        return Err(ForgeError::RateLimited(
            "Too many login attempts. Please wait 1 minute before retrying.".into(),
        ));
    }

    // Find by username or email
    let user = if login_str.contains('@') {
        state.db.get_user_by_email(login_str).await?
    } else {
        state.db.get_user_by_username(login_str).await?
    };

    let user = match user {
        Some(u) => u,
        None => {
            // Generic error message to prevent user enumeration
            return Err(ForgeError::Unauthorized(
                "Invalid username or password".into(),
            ));
        }
    };

    let valid = verify_password(&payload.password, &user.password_hash)?;
    if !valid {
        return Err(ForgeError::Unauthorized(
            "Invalid username or password".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let _ = state.db.update_user_last_login(&user.id, now).await;

    let token = generate_session_token();
    let token_hash = hash_session_token(&token);
    let expires_at = now + DEFAULT_SESSION_TTL_NANOS;

    let session = UserSession {
        id: format!("sess_{}", uuid::Uuid::new_v4().simple()),
        user_id: user.id.clone(),
        session_token_hash: token_hash,
        ip_address: None,
        user_agent: None,
        created_at_nanos: now,
        expires_at_nanos: expires_at,
        revoked: false,
    };
    state.db.create_session(session).await?;

    let secure_flag = if state.public_origin.starts_with("https://")
        || state.profile == crate::core::profile::DeploymentProfile::CoreForge
    {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "spine_session={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=604800{}",
        token, secure_flag
    );
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());

    let resp = AuthSuccessResponse {
        user: UserDto::from(&user),
        token,
        expires_at_nanos: expires_at,
    };

    Ok((StatusCode::OK, headers, Json(resp)))
}

pub async fn logout(
    State(state): State<AppState>,
    session: Option<Extension<UserSession>>,
) -> Result<impl IntoResponse, ForgeError> {
    if let Some(Extension(sess)) = session {
        state.db.revoke_session(&sess.id).await?;
    }

    let secure_flag = if state.public_origin.starts_with("https://")
        || state.profile == crate::core::profile::DeploymentProfile::CoreForge
    {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "spine_session=; HttpOnly; SameSite=Lax; Path=/; Max-Age=0{}",
        secure_flag
    );
    let mut headers = HeaderMap::new();
    headers.insert(header::SET_COOKIE, HeaderValue::from_str(&cookie).unwrap());

    Ok((
        StatusCode::OK,
        headers,
        Json(GenericMessageResponse {
            message: "Successfully logged out",
        }),
    ))
}

pub async fn get_session(
    user: Option<Extension<User>>,
    session: Option<Extension<UserSession>>,
) -> Result<impl IntoResponse, ForgeError> {
    match (user, session) {
        (Some(Extension(u)), Some(Extension(s))) => Ok(Json(serde_json::json!({
            "authenticated": true,
            "user": UserDto::from(&u),
            "session": SessionDto {
                id: s.id,
                ip_address: s.ip_address,
                user_agent: s.user_agent,
                created_at_nanos: s.created_at_nanos,
                expires_at_nanos: s.expires_at_nanos,
                is_active: true,
            }
        }))),
        _ => Ok(Json(serde_json::json!({
            "authenticated": false,
            "user": null,
            "session": null
        }))),
    }
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(user): Extension<User>,
    Json(payload): Json<ChangePasswordRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let valid = verify_password(&payload.current_password, &user.password_hash)?;
    if !valid {
        return Err(ForgeError::Unauthorized(
            "Current password is incorrect".into(),
        ));
    }

    validate_password_strength(&payload.new_password)?;
    let new_hash = hash_password(&payload.new_password)?;
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    state
        .db
        .update_user_password(&user.id, &new_hash, now)
        .await?;
    // Revoke other existing sessions
    state.db.revoke_all_user_sessions(&user.id).await?;

    Ok(Json(GenericMessageResponse {
        message: "Password changed successfully. Please log in with your new password.",
    }))
}

pub async fn request_password_reset(
    State(state): State<AppState>,
    Json(payload): Json<ResetPasswordRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let email = payload.email.trim();
    if !state
        .rate_limiter
        .check_and_record(&format!("reset_{}", email.to_lowercase()), 3, 300)
    {
        return Err(ForgeError::RateLimited(
            "Too many password reset requests. Please wait a few minutes.".into(),
        ));
    }

    if let Ok(Some(user)) = state.db.get_user_by_email(email).await {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let token = generate_reset_token();
        let token_hash = hash_token(&token);
        let _ = state
            .db
            .create_reset_token(&token_hash, &user.id, now + 3600 * 1_000_000_000)
            .await;
        let _ = state
            .mail
            .send_email(
                &user.email,
                "Reset your Spine Forge password",
                &format!("Your password reset token is: {}", token),
            )
            .await;
    }

    // Always return generic success to avoid email enumeration
    Ok(Json(GenericMessageResponse {
        message: "If that email is registered, password reset instructions have been sent.",
    }))
}

pub async fn complete_password_reset(
    State(state): State<AppState>,
    Json(payload): Json<ResetPasswordCompleteRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    validate_password_strength(&payload.new_password)?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let token_hash = hash_token(payload.token.trim());

    let user_id = state
        .db
        .consume_reset_token(&token_hash, now)
        .await?
        .ok_or_else(|| ForgeError::BadRequest("Invalid or expired password reset token".into()))?;

    let new_hash = hash_password(&payload.new_password)?;
    state
        .db
        .update_user_password(&user_id, &new_hash, now)
        .await?;
    state.db.revoke_all_user_sessions(&user_id).await?;

    Ok(Json(GenericMessageResponse {
        message: "Password reset completed successfully. You may now log in.",
    }))
}

pub async fn verify_email(
    State(state): State<AppState>,
    Json(payload): Json<VerifyEmailRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let token_hash = hash_token(payload.token.trim());

    let user_id = state
        .db
        .consume_email_token(&token_hash, now)
        .await?
        .ok_or_else(|| ForgeError::BadRequest("Invalid or expired verification token".into()))?;

    let audit = AuditEvent {
        id: 0,
        user_id: Some(user_id),
        action: "user.email_verified".to_string(),
        resource_type: "user".to_string(),
        resource_id: "self".to_string(),
        ip_address: None,
        metadata: serde_json::json!({}),
        timestamp_nanos: now,
    };
    let _ = state.db.record_audit_event(audit).await;

    Ok(Json(GenericMessageResponse {
        message: "Email verified successfully.",
    }))
}
