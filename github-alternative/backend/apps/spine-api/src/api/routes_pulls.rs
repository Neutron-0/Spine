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
use spine_cluster::PrProtectionContext;

#[derive(Deserialize)]
pub struct ListPullsQuery {
    pub state: Option<String>,
}

#[derive(Deserialize)]
pub struct CreatePullRequestRequest {
    pub title: String,
    pub body: Option<String>,
    pub base_branch: String,
    pub head_branch: String,
    pub is_draft: Option<bool>,
}

#[derive(Deserialize)]
pub struct UpdatePullRequestRequest {
    pub title: Option<String>,
    pub body: Option<String>,
    pub state: Option<String>, // OPEN or CLOSED
}

#[derive(Deserialize)]
pub struct MergePullRequestRequest {
    pub commit_message: Option<String>,
    pub merge_strategy: Option<String>,
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

pub async fn list_pull_requests(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Query(query): Query<ListPullsQuery>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state.repositories.get_repository(caller, &owner, &repo_name).await?;

    let prs = state
        .db
        .list_pull_requests_for_repo(&repo.id, query.state.as_deref())
        .await?;

    let mut dtos = Vec::new();
    for pr in prs {
        let author = state
            .db
            .get_user_by_id(&pr.author_id)
            .await?
            .map(|u| UserDto::from(&u))
            .unwrap_or_else(|| fallback_user_dto(&pr.author_id));

        let reviewer_ids = state.db.list_pr_requested_reviewers(&pr.id).await?;
        let mut requested_reviewers = Vec::new();
        for rid in reviewer_ids {
            if let Some(u) = state.db.get_user_by_id(&rid).await? {
                requested_reviewers.push(UserDto::from(&u));
            }
        }

        let reviews_count = state.db.list_pr_reviews(&pr.id).await?.len();
        let comments_count = state.db.list_pr_review_comments(&pr.id).await?.len();

        dtos.push(PullRequestDto {
            id: pr.id,
            repo_id: repo.id.clone(),
            pr_number: pr.pr_number,
            author,
            title: pr.title,
            body: pr.body,
            base_branch: pr.base_branch,
            head_branch: pr.head_branch,
            base_commit_id: pr.base_commit_id,
            head_commit_id: pr.head_commit_id,
            state: pr.state,
            is_draft: pr.is_draft,
            merge_commit_id: pr.merge_commit_id,
            mergeable: None,
            requested_reviewers,
            reviews_count,
            comments_count,
            created_at_nanos: pr.created_at_nanos,
            updated_at_nanos: pr.updated_at_nanos,
            closed_at_nanos: pr.closed_at_nanos,
            merged_at_nanos: pr.merged_at_nanos,
        });
    }

    Ok(Json(dtos))
}

pub async fn create_pull_request(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name)): Path<(String, String)>,
    Json(payload): Json<CreatePullRequestRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to create PR".into()))?;
    let repo = state.repositories.get_repository(Some(&caller), &owner, &repo_name).await?;

    let title = payload.title.trim().to_string();
    if title.is_empty() {
        return Err(ForgeError::BadRequest("PR title cannot be empty".into()));
    }

    let base_branch = payload.base_branch.trim().to_string();
    let head_branch = payload.head_branch.trim().to_string();
    if base_branch == head_branch {
        return Err(ForgeError::BadRequest("Base branch and head branch cannot be identical".into()));
    }

    // Resolve commits from native storage
    let base_oid = state.vcs.resolve_ref(&repo.native_repo_id, &base_branch)?;
    let head_oid = state.vcs.resolve_ref(&repo.native_repo_id, &head_branch)?;

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let pr_id = format!("pr_{}", uuid::Uuid::new_v4().simple());

    let pr = PullRequest {
        id: pr_id.clone(),
        repo_id: repo.id.clone(),
        pr_number: 0, // Sequential in DB
        author_id: caller.id.clone(),
        title: title.clone(),
        body: payload.body.unwrap_or_default(),
        base_branch,
        head_branch,
        base_commit_id: base_oid.to_hex(),
        head_commit_id: head_oid.to_hex(),
        state: "OPEN".to_string(),
        is_draft: payload.is_draft.unwrap_or(false),
        merged_by_user_id: None,
        merge_commit_id: None,
        merge_strategy: Some("MERGE_COMMIT".to_string()),
        closed_at_nanos: None,
        merged_at_nanos: None,
        created_at_nanos: now,
        updated_at_nanos: now,
    };

    let created = state.db.create_pull_request(pr).await?;

    // Activity event
    let _ = state.db.record_activity_event(RepositoryActivityEvent {
        id: format!("act_{}", uuid::Uuid::new_v4().simple()),
        repo_id: repo.id.clone(),
        actor_id: Some(caller.id.clone()),
        event_type: "pull_request.opened".to_string(),
        payload: serde_json::json!({
            "pr_number": created.pr_number,
            "title": created.title,
            "base": created.base_branch,
            "head": created.head_branch,
        }),
        created_at_nanos: now,
    }).await;

    Ok((StatusCode::CREATED, Json(created)))
}

pub async fn get_pull_request(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state.repositories.get_repository(caller, &owner, &repo_name).await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let author = state
        .db
        .get_user_by_id(&pr.author_id)
        .await?
        .map(|u| UserDto::from(&u))
        .unwrap_or_else(|| fallback_user_dto(&pr.author_id));

    let reviewer_ids = state.db.list_pr_requested_reviewers(&pr.id).await?;
    let mut requested_reviewers = Vec::new();
    for rid in reviewer_ids {
        if let Some(u) = state.db.get_user_by_id(&rid).await? {
            requested_reviewers.push(UserDto::from(&u));
        }
    }

    let reviews_count = state.db.list_pr_reviews(&pr.id).await?.len();
    let comments_count = state.db.list_pr_review_comments(&pr.id).await?.len();

    let dto = PullRequestDto {
        id: pr.id,
        repo_id: repo.id.clone(),
        pr_number: pr.pr_number,
        author,
        title: pr.title,
        body: pr.body,
        base_branch: pr.base_branch,
        head_branch: pr.head_branch,
        base_commit_id: pr.base_commit_id,
        head_commit_id: pr.head_commit_id,
        state: pr.state,
        is_draft: pr.is_draft,
        merge_commit_id: pr.merge_commit_id,
        mergeable: None,
        requested_reviewers,
        reviews_count,
        comments_count,
        created_at_nanos: pr.created_at_nanos,
        updated_at_nanos: pr.updated_at_nanos,
        closed_at_nanos: pr.closed_at_nanos,
        merged_at_nanos: pr.merged_at_nanos,
    };

    Ok(Json(dto))
}

pub async fn update_pull_request(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<UpdatePullRequestRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required".into()))?;
    let repo = state.repositories.get_repository(Some(&caller), &owner, &repo_name).await?;

    let mut pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let is_author = pr.author_id == caller.id;
    let is_collaborator = state.permissions.check_write_access(Some(&caller), &repo).await.is_ok();
    if !is_author && !is_collaborator {
        return Err(ForgeError::Forbidden("Only PR author or repository collaborators can edit PR".into()));
    }

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    if let Some(t) = payload.title {
        pr.title = t;
    }
    if let Some(b) = payload.body {
        pr.body = b;
    }
    if let Some(st) = payload.state {
        let upper = st.to_uppercase();
        if upper == "CLOSED" && pr.state == "OPEN" {
            pr.state = "CLOSED".to_string();
            pr.closed_at_nanos = Some(now);
        } else if upper == "OPEN" && pr.state == "CLOSED" {
            pr.state = "OPEN".to_string();
            pr.closed_at_nanos = None;
        }
    }
    pr.updated_at_nanos = now;

    let updated = state.db.update_pull_request(pr).await?;
    Ok(Json(updated))
}

pub async fn get_pull_request_diff(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state.repositories.get_repository(caller, &owner, &repo_name).await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let diff = state.vcs.compare_branches(&repo.native_repo_id, &pr.base_branch, &pr.head_branch)?;
    Ok(Json(diff))
}

pub async fn get_pull_request_mergeability(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.as_ref().map(|Extension(u)| u);
    let repo = state.repositories.get_repository(caller, &owner, &repo_name).await?;

    let pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    let report = state.vcs.check_mergeability(&repo.native_repo_id, &pr.base_branch, &pr.head_branch)?;
    Ok(Json(report))
}

pub async fn execute_pull_request_merge(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Path((owner, repo_name, number)): Path<(String, String, i64)>,
    Json(payload): Json<MergePullRequestRequest>,
) -> Result<impl IntoResponse, ForgeError> {
    let caller = user.ok_or_else(|| ForgeError::Unauthorized("Authentication required to merge PR".into()))?;
    let repo = state.repositories.get_repository(Some(&caller), &owner, &repo_name).await?;

    let mut pr = state
        .db
        .get_pull_request_by_number(&repo.id, number)
        .await?
        .ok_or_else(|| ForgeError::NotFound("Pull request not found".into()))?;

    if pr.state != "OPEN" {
        return Err(ForgeError::BadRequest(format!("Cannot merge PR in '{}' state", pr.state)));
    }

    // Branch protection checks
    state.permissions.check_merge_allowed(&caller, &repo, &pr.base_branch, &pr).await?;

    let commit_message = payload.commit_message.unwrap_or_else(|| {
        format!("Merge pull request #{} from {}\n\n{}", pr.pr_number, pr.head_branch, pr.title)
    });

    // Check cross-service crash reconciliation
    let current_base_oid = state.vcs.resolve_ref(&repo.native_repo_id, &pr.base_branch).ok();
    let mut already_merged = false;
    if let Some(cur_oid) = current_base_oid {
        if let Ok(commit) = state.vcs.get_commit(&repo.native_repo_id, &cur_oid.to_hex()) {
            if commit.parents.len() == 2 && commit.parents[0] == pr.base_commit_id && commit.parents[1] == pr.head_commit_id {
                already_merged = true;
            }
        }
    }

    let merge_commit_id = if already_merged {
        current_base_oid.unwrap().to_hex()
    } else {
        let reviews = state.db.list_pr_reviews(&pr.id).await.unwrap_or_default();
        let comments = state.db.list_pr_review_comments(&pr.id).await.unwrap_or_default();
        let approved_count = reviews.iter().filter(|r| r.state == "APPROVED" && r.reviewer_id != pr.author_id).count() as u32;
        let has_active_change_requests = reviews.iter().any(|r| r.state == "CHANGES_REQUESTED");
        let has_unresolved_comments = comments.iter().any(|c| !c.is_resolved);

        let pr_context = PrProtectionContext {
            pr_number: pr.pr_number as u64,
            author_id: pr.author_id.clone(),
            approved_count,
            has_active_change_requests,
            has_unresolved_comments,
        };

        let merge_result = state.vcs.execute_merge(
            &repo.native_repo_id,
            &pr.base_branch,
            &pr.head_branch,
            &caller.id,
            &caller.display_name,
            &caller.email,
            &commit_message,
            Some(&pr_context),
        ).await?;

        merge_result.merge_commit_id.unwrap_or_default()
    };

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    pr.state = "MERGED".to_string();
    pr.merged_by_user_id = Some(caller.id.clone());
    pr.merge_commit_id = Some(merge_commit_id.clone());
    pr.merged_at_nanos = Some(now);
    pr.closed_at_nanos = Some(now);
    pr.updated_at_nanos = now;

    state.db.update_pull_request(pr.clone()).await?;

    // Auto-close issues referenced with 'Fixes #X' or 'Closes #X' in PR description
    let combined_text = format!("{}\n{}", pr.title, pr.body);
    for line in combined_text.lines() {
        let lower = line.to_lowercase();
        for trigger in &["fixes #", "closes #", "resolves #"] {
            if let Some(pos) = lower.find(trigger) {
                let rest = &lower[pos + trigger.len()..];
                let num_str: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(num) = num_str.parse::<i64>() {
                    if let Ok(Some(mut issue)) = state.db.get_issue_by_number(&repo.id, num).await {
                        if issue.status != "CLOSED" {
                            issue.status = "CLOSED".to_string();
                            issue.closed_by_user_id = Some(caller.id.clone());
                            issue.closed_at_nanos = Some(now);
                            issue.updated_at_nanos = now;
                            let _ = state.db.update_issue(issue).await;
                        }
                    }
                }
            }
        }
    }

    // Notify PR author
    if pr.author_id != caller.id {
        let notif = UserNotification {
            id: format!("notif_{}", uuid::Uuid::new_v4().simple()),
            user_id: pr.author_id.clone(),
            reason: "PR_MERGED".to_string(),
            resource_type: "pull_request".to_string(),
            resource_id: pr.id.clone(),
            title: format!("Pull request #{} merged", pr.pr_number),
            message: format!("{} merged your pull request: {}", caller.username, pr.title),
            is_read: false,
            created_at_nanos: now,
        };
        let _ = state.db.create_notification(notif).await;
    }

    // Activity event
    let _ = state.db.record_activity_event(RepositoryActivityEvent {
        id: format!("act_{}", uuid::Uuid::new_v4().simple()),
        repo_id: repo.id.clone(),
        actor_id: Some(caller.id.clone()),
        event_type: "pull_request.merged".to_string(),
        payload: serde_json::json!({
            "pr_number": pr.pr_number,
            "title": pr.title,
            "merge_commit_id": merge_commit_id,
        }),
        created_at_nanos: now,
    }).await;

    Ok(Json(MergeResultDto {
        success: true,
        merge_commit_id: Some(merge_commit_id),
        message: format!("Successfully merged {} into {}", pr.head_branch, pr.base_branch),
    }))
}
