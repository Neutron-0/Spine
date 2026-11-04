use super::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::*;
use axum::{
    extract::{Extension, Path, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;

#[derive(Deserialize)]
pub struct CreateReviewRequest {
    pub commit_id: Option<String>,
    pub state: String, // APPROVED, CHANGES_REQUESTED, COMMENTED
    pub body: Option<String>,
}

#[derive(Deserialize)]
pub struct RequestReviewerRequest {
    pub reviewer_username: String,
}

#[derive(Deserialize)]
pub struct CreateReviewCommentRequest {
    pub review_id: Option<String>,
    pub file_path: String,
    pub commit_id: Option<String>,
    pub diff_side: Option<String>, // LEFT or RIGHT
    pub line_number: i32,
    pub body: String,
}

#[derive(Deserialize)]
pub struct ResolveCommentRequest {
    pub resolved: bool,
}

fn fallback_user_dto(id: &str) -> UserDto {
    UserDto {
        id: id.to_string(),
        username: "unknown".into(),
        email: "".into(),
        display_name: "Unknown".into(),
        account_status: "ACTIVE".into(),
        avatar_url: None,
        created_at_nanos: 0,
    }
}

pub async fn list_reviews(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let reviews = state.db.list_pr_reviews(&pr.id).await?;
    let mut dtos = Vec::new();
    for rev in reviews {
        let reviewer = state
            .db
            .get_user_by_id(&rev.reviewer_id)
            .await?
            .map(|u| UserDto::from(&u))
            .unwrap_or_else(|| fallback_user_dto(&rev.reviewer_id));

        dtos.push(PullRequestReviewDto {
            id: rev.id,
            reviewer,
            commit_id: rev.commit_id,
            state: rev.state,
            body: rev.body,
            created_at_nanos: rev.created_at_nanos,
        });
    }

    Ok(Json(dtos))
}

pub async fn create_review(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<CreateReviewRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to review".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let rev_state = payload.state.trim().to_uppercase();
    if rev_state != "APPROVED" && rev_state != "CHANGES_REQUESTED" && rev_state != "COMMENTED" {
        return Err(ForgeError::BadRequest(
            "Review state must be APPROVED, CHANGES_REQUESTED, or COMMENTED".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let review_id = format!("rev_{}", uuid::Uuid::new_v4().simple());
    let commit_id = payload
        .commit_id
        .unwrap_or_else(|| pr.head_commit_id.clone());

    let review = PullRequestReview {
        id: review_id.clone(),
        pr_id: pr.id.clone(),
        reviewer_id: caller.id.clone(),
        commit_id: commit_id.clone(),
        state: rev_state.clone(),
        body: payload.body.unwrap_or_default(),
        dismissed_by_user_id: None,
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_pr_review(review).await?;

    // Notify PR author
    if pr.author_id != caller.id {
        let notif = UserNotification {
            id: format!("notif_{}", uuid::Uuid::new_v4().simple()),
            user_id: pr.author_id.clone(),
            reason: "PR_REVIEW".to_string(),
            resource_type: "pull_request".to_string(),
            resource_id: pr.id.clone(),
            title: format!("New review on PR #{}: {}", pr.pr_number, rev_state),
            message: format!(
                "{} reviewed your pull request with status {}",
                caller.username, rev_state
            ),
            is_read: false,
            created_at_nanos: now,
        };
        let _ = state.db.create_notification(notif).await;
    }

    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn dismiss_review(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, _number, review_id)): Path<(String, String, i64, String)>,
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

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    state
        .db
        .dismiss_pr_review(&review_id, &caller.id, now)
        .await?;
    Ok(StatusCode::OK)
}

pub async fn request_reviewer(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<RequestReviewerRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let reviewer = state
        .db
        .get_user_by_username(&payload.reviewer_username)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Reviewer user not found".into()))?;

    state
        .db
        .request_pr_reviewer(&pr.id, &reviewer.id, &caller.id)
        .await?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let notif = UserNotification {
        id: format!("notif_{}", uuid::Uuid::new_v4().simple()),
        user_id: reviewer.id.clone(),
        reason: "REVIEW_REQUESTED".to_string(),
        resource_type: "pull_request".to_string(),
        resource_id: pr.id.clone(),
        title: format!("Review requested on PR #{}", pr.pr_number),
        message: format!(
            "{} requested your review on PR: {}",
            caller.username, pr.title
        ),
        is_read: false,
        created_at_nanos: now,
    };
    let _ = state.db.create_notification(notif).await;

    Ok(StatusCode::OK)
}

pub async fn list_review_comments(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let comments = state.db.list_pr_review_comments(&pr.id).await?;
    let mut dtos = Vec::new();
    for c in comments {
        let author = state
            .db
            .get_user_by_id(&c.author_id)
            .await?
            .map(|u| UserDto::from(&u))
            .unwrap_or_else(|| fallback_user_dto(&c.author_id));

        let is_outdated = c.commit_id != pr.head_commit_id;

        dtos.push(PullRequestReviewCommentDto {
            id: c.id,
            author,
            file_path: c.file_path,
            commit_id: c.commit_id,
            diff_side: c.diff_side,
            line_number: c.line_number,
            body: c.body,
            is_resolved: c.is_resolved,
            is_outdated,
            created_at_nanos: c.created_at_nanos,
        });
    }

    Ok(Json(dtos))
}

pub async fn create_review_comment(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<CreateReviewCommentRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller =
        user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to comment".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let body = payload.body.trim().to_string();
    if body.is_empty() {
        return Err(ForgeError::BadRequest(
            "Comment body cannot be empty".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let comment_id = format!("prc_{}", uuid::Uuid::new_v4().simple());
    let commit_id = payload
        .commit_id
        .unwrap_or_else(|| pr.head_commit_id.clone());

    let comment = PullRequestReviewComment {
        id: comment_id.clone(),
        pr_id: pr.id.clone(),
        review_id: payload.review_id,
        author_id: caller.id.clone(),
        file_path: payload.file_path,
        commit_id,
        diff_side: payload.diff_side.unwrap_or_else(|| "RIGHT".to_string()),
        line_number: payload.line_number,
        body,
        is_resolved: false,
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_pr_review_comment(comment).await?;
    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn resolve_review_comment(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, _number, comment_id)): Path<(String, String, i64, String)>,
    Json(payload): Json<ResolveCommentRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    state
        .db
        .resolve_pr_review_comment(&comment_id, payload.resolved)
        .await?;
    Ok(StatusCode::OK)
}
