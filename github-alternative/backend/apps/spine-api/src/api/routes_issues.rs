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
pub struct ListIssuesQuery {
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateIssueRequest {
    pub title: String,
    pub body: Option<String>,
    pub milestone_id: Option<String>,
    pub assignees: Option<Vec<String>>,
    pub labels: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct UpdateIssueRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub status: Option<String>, // OPEN or CLOSED
    pub milestone_id: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateCommentRequest {
    pub body: String,
}

#[derive(Deserialize)]
pub struct CreateLabelRequest {
    pub name: String,
    pub color: Option<String>,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct CreateMilestoneRequest {
    pub title: String,
    pub description: Option<String>,
    pub due_date_nanos: Option<i64>,
}

#[derive(Deserialize)]
pub struct UpdateMilestoneRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub state: Option<String>, // OPEN or CLOSED
    pub due_date_nanos: Option<i64>,
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

pub async fn list_issues(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<ListIssuesQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let issues = state
        .db
        .list_issues_for_repo(&repo.id, query.status.as_deref())
        .await?;

    let mut dtos = Vec::new();
    for issue in issues {
        let author = state
            .db
            .get_user_by_id(&issue.author_id)
            .await?
            .map(|u| UserDto::from(&u))
            .unwrap_or_else(|| fallback_user_dto(&issue.author_id));

        let assignee_ids = state.db.list_issue_assignees(&issue.id).await?;
        let mut assignees = Vec::new();
        for aid in assignee_ids {
            if let Some(u) = state.db.get_user_by_id(&aid).await? {
                assignees.push(UserDto::from(&u));
            }
        }

        let label_ids = state.db.list_issue_labels(&issue.id).await?;
        let repo_labels = state.db.list_labels_for_repo(&repo.id).await?;
        let labels: Vec<LabelDto> = repo_labels
            .into_iter()
            .filter(|l| label_ids.contains(&l.id))
            .map(|l| LabelDto::from(&l))
            .collect();

        let milestone = if let Some(mid) = &issue.milestone_id {
            state
                .db
                .get_milestone_by_id(mid)
                .await?
                .map(|m| MilestoneDto::from(&m))
        } else {
            None
        };

        let comments_count = state.db.list_issue_comments(&issue.id).await?.len();

        dtos.push(IssueDto {
            id: issue.id,
            repo_id: repo.id.clone(),
            issue_number: issue.issue_number,
            author,
            title: issue.title,
            body: issue.body,
            status: issue.status,
            assignees,
            labels,
            milestone,
            comments_count,
            created_at_nanos: issue.created_at_nanos,
            updated_at_nanos: issue.updated_at_nanos,
            closed_at_nanos: issue.closed_at_nanos,
        });
    }

    Ok(Json(dtos))
}

pub async fn create_issue(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateIssueRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| {
        ForgeError::Unauthorized("Authentication required to create issue".into())
    })?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return Err(ForgeError::BadRequest("Issue title cannot be empty".into()));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let issue_id = format!("iss_{}", uuid::Uuid::new_v4().simple());

    let issue = Issue {
        id: issue_id.clone(),
        repo_id: repo.id.clone(),
        issue_number: 0, // Will be computed sequentially by DB
        author_id: caller.id.clone(),
        title: title.clone(),
        body: payload.body.unwrap_or_default(),
        status: "OPEN".to_string(),
        milestone_id: payload.milestone_id,
        closed_by_user_id: None,
        closed_at_nanos: None,
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_issue(issue).await?;

    // Handle assignees if provided
    if let Some(usernames) = payload.assignees {
        for uname in usernames {
            if let Some(target) = state.db.get_user_by_username(&uname).await? {
                let _ = state.db.assign_issue(&created.id, &target.id).await;
            }
        }
    }

    // Handle labels if provided
    if let Some(lbl_ids) = payload.labels {
        for lid in lbl_ids {
            let _ = state.db.add_issue_label(&created.id, &lid).await;
        }
    }

    // Record activity
    let _ = state
        .db
        .record_activity_event(RepositoryActivityEvent {
            id: format!("act_{}", uuid::Uuid::new_v4().simple()),
            repo_id: repo.id.clone(),
            actor_id: Some(caller.id.clone()),
            event_type: "issue.opened".to_string(),
            payload: serde_json::json!({
                "issue_number": created.issue_number,
                "title": created.title,
            }),
            created_at_nanos: now,
        })
        .await;

    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn get_issue(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let issue = state
        .db
        .get_issue_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Issue not found".into()))?;

    let author = state
        .db
        .get_user_by_id(&issue.author_id)
        .await?
        .map(|u| UserDto::from(&u))
        .unwrap_or_else(|| fallback_user_dto(&issue.author_id));

    let assignee_ids = state.db.list_issue_assignees(&issue.id).await?;
    let mut assignees = Vec::new();
    for aid in assignee_ids {
        if let Some(u) = state.db.get_user_by_id(&aid).await? {
            assignees.push(UserDto::from(&u));
        }
    }

    let label_ids = state.db.list_issue_labels(&issue.id).await?;
    let repo_labels = state.db.list_labels_for_repo(&repo.id).await?;
    let labels: Vec<LabelDto> = repo_labels
        .into_iter()
        .filter(|l| label_ids.contains(&l.id))
        .map(|l| LabelDto::from(&l))
        .collect();

    let milestone = if let Some(mid) = &issue.milestone_id {
        state
            .db
            .get_milestone_by_id(mid)
            .await?
            .map(|m| MilestoneDto::from(&m))
    } else {
        None
    };

    let comments_count = state.db.list_issue_comments(&issue.id).await?.len();

    let dto = IssueDto {
        id: issue.id,
        repo_id: repo.id.clone(),
        issue_number: issue.issue_number,
        author,
        title: issue.title,
        body: issue.body,
        status: issue.status,
        assignees,
        labels,
        milestone,
        comments_count,
        created_at_nanos: issue.created_at_nanos,
        updated_at_nanos: issue.updated_at_nanos,
        closed_at_nanos: issue.closed_at_nanos,
    };

    Ok(Json(dto))
}

pub async fn update_issue(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<UpdateIssueRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let mut issue = state
        .db
        .get_issue_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Issue not found".into()))?;

    // Only author or collaborator can update
    let is_author = issue.author_id == caller.id;
    let is_collaborator = state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await
        .is_ok();
    if !is_author && !is_collaborator {
        return Err(ForgeError::Forbidden(
            "Only issue author or repository collaborators can edit issue".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    if let Some(t) = payload.title {
        issue.title = t;
    }
    if let Some(b) = payload.body {
        issue.body = b;
    }
    if let Some(m) = payload.milestone_id {
        issue.milestone_id = if m.is_empty() { None } else { Some(m) };
    }
    if let Some(st) = payload.status {
        let upper = st.to_uppercase();
        if upper == "CLOSED" && issue.status != "CLOSED" {
            issue.status = "CLOSED".to_string();
            issue.closed_by_user_id = Some(caller.id.clone());
            issue.closed_at_nanos = Some(now);
        } else if upper == "OPEN" && issue.status != "OPEN" {
            issue.status = "OPEN".to_string();
            issue.closed_by_user_id = None;
            issue.closed_at_nanos = None;
        }
    }
    issue.updated_at_nanos = now;

    let updated = state.db.update_issue(issue).await?;
    Ok(Json(updated))
}

pub async fn list_issue_comments(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let issue = state
        .db
        .get_issue_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Issue not found".into()))?;

    let comments = state.db.list_issue_comments(&issue.id).await?;
    let mut dtos = Vec::new();
    for c in comments {
        let author = state
            .db
            .get_user_by_id(&c.author_id)
            .await?
            .map(|u| UserDto::from(&u))
            .unwrap_or_else(|| fallback_user_dto(&c.author_id));

        dtos.push(IssueCommentDto {
            id: c.id,
            author,
            body: c.body,
            created_at_nanos: c.created_at_nanos,
            updated_at_nanos: c.updated_at_nanos,
        });
    }

    Ok(Json(dtos))
}

pub async fn create_issue_comment(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<CreateCommentRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;

    let issue = state
        .db
        .get_issue_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Issue not found".into()))?;

    let body = payload.body.trim().to_string();
    if body.is_empty() {
        return Err(ForgeError::BadRequest(
            "Comment body cannot be empty".into(),
        ));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let comment = IssueComment {
        id: format!("ic_{}", uuid::Uuid::new_v4().simple()),
        issue_id: issue.id.clone(),
        author_id: caller.id.clone(),
        body: body.clone(),
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_issue_comment(comment).await?;

    // Notify issue author if different from commenter
    if issue.author_id != caller.id {
        let notif = UserNotification {
            id: format!("notif_{}", uuid::Uuid::new_v4().simple()),
            user_id: issue.author_id.clone(),
            reason: "ISSUE_COMMENT".to_string(),
            resource_type: "issue".to_string(),
            resource_id: issue.id.clone(),
            title: format!("New comment on issue #{}", issue.issue_number),
            message: format!("{} commented: {}", caller.username, body),
            is_read: false,
            created_at_nanos: now,
        };
        let _ = state.db.create_notification(notif).await;
    }

    Ok((StatusCode::CREATED, Json(created)))
}

// Labels
pub async fn list_labels(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let labels = state.db.list_labels_for_repo(&repo.id).await?;
    let dtos: Vec<LabelDto> = labels.iter().map(LabelDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_label(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateLabelRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let label = RepositoryLabel {
        id: format!("lbl_{}", uuid::Uuid::new_v4().simple()),
        repo_id: repo.id.clone(),
        name: payload.name.trim().to_string(),
        color: payload.color.unwrap_or_else(|| "#58a6ff".to_string()),
        description: payload.description.unwrap_or_default(),
        created_at_nanos: now,
    };

    let created = state.db.create_label(label).await?;
    Ok((StatusCode::CREATED, Json(LabelDto::from(&created))))
}

pub async fn delete_label(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, label_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let repo_labels = state.db.list_labels_for_repo(&repo.id).await?;
    if !repo_labels.iter().any(|l| l.id == label_id) {
        return Err(ForgeError::NotFound("Label not found in repository".into()));
    }

    state.db.delete_label(&label_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// Milestones
pub async fn list_milestones(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state
        .repositories
        .get_repository(caller, &owner, &repo_name)
        .await?;

    let milestones = state.db.list_milestones_for_repo(&repo.id).await?;
    let dtos: Vec<MilestoneDto> = milestones.iter().map(MilestoneDto::from).collect();
    Ok(Json(dtos))
}

pub async fn create_milestone(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreateMilestoneRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let milestone = Milestone {
        id: format!("ms_{}", uuid::Uuid::new_v4().simple()),
        repo_id: repo.id.clone(),
        title: payload.title.trim().to_string(),
        description: payload.description.unwrap_or_default(),
        state: "OPEN".to_string(),
        due_date_nanos: payload.due_date_nanos,
        created_at_nanos: now,
        updated_at_nanos: now,
        closed_at_nanos: None,
    };

    let created = state.db.create_milestone(milestone).await?;
    Ok((StatusCode::CREATED, Json(MilestoneDto::from(&created))))
}

pub async fn update_milestone(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, milestone_id)): Path<(String, String, String)>,
    Json(payload): Json<UpdateMilestoneRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let mut milestone = state
        .db
        .get_milestone_by_id(&milestone_id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Milestone not found".into()))?;

    if milestone.repo_id != repo.id {
        return Err(ForgeError::NotFound("Milestone not found in repository".into()));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    if let Some(t) = payload.title {
        milestone.title = t;
    }
    if let Some(d) = payload.description {
        milestone.description = d;
    }
    if let Some(due) = payload.due_date_nanos {
        milestone.due_date_nanos = Some(due);
    }
    if let Some(st) = payload.state {
        let upper = st.to_uppercase();
        if upper == "CLOSED" {
            milestone.state = "CLOSED".to_string();
            milestone.closed_at_nanos = Some(now);
        } else {
            milestone.state = "OPEN".to_string();
            milestone.closed_at_nanos = None;
        }
    }
    milestone.updated_at_nanos = now;

    let updated = state.db.update_milestone(milestone).await?;
    Ok(Json(MilestoneDto::from(&updated)))
}

pub async fn delete_milestone(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, milestone_id)): Path<(String, String, String)>,
) -> Result<impl IntoResponse, ForgeError> {
    let _permit = state.consistency_barrier.acquire_write_permit().await?;
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &repo_name)
        .await?;
    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await?;

    let milestone = state
        .db
        .get_milestone_by_id(&milestone_id)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Milestone not found".into()))?;

    if milestone.repo_id != repo.id {
        return Err(ForgeError::NotFound("Milestone not found in repository".into()));
    }

    state.db.delete_milestone(&milestone_id).await?;
    Ok(StatusCode::NO_CONTENT)
}
