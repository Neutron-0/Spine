use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;

fn make_test_user(id: &str, username: &str) -> User {
    User {
        id: id.to_string(),
        username: username.to_string(),
        normalized_username: username.to_lowercase(),
        email: format!("{}@spine.build", username),
        normalized_email: format!("{}@spine.build", username),
        display_name: username.to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
        last_login_nanos: None,
    }
}

#[tokio::test]
async fn test_issues_labels_and_milestones() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let author = make_test_user("usr_alice", "alice");
    let assignee = make_test_user("usr_bob", "bob");
    db.create_user(author.clone()).await.unwrap();
    db.create_user(assignee.clone()).await.unwrap();

    let repo_id = "repo_test";

    // 1. Create Milestone
    let ms = Milestone {
        id: "ms_v1".into(),
        repo_id: repo_id.into(),
        title: "v1.0 Launch".into(),
        description: "Initial production milestone".into(),
        state: "OPEN".into(),
        due_date_nanos: Some(2_000_000_000),
        created_at_nanos: 100,
        updated_at_nanos: 100,
        closed_at_nanos: None,
    };
    db.create_milestone(ms.clone()).await.unwrap();

    // 2. Create Label
    let label = RepositoryLabel {
        id: "lbl_bug".into(),
        repo_id: repo_id.into(),
        name: "bug".into(),
        color: "#d73a4a".into(),
        description: "Something isn't working".into(),
        created_at_nanos: 100,
    };
    db.create_label(label.clone()).await.unwrap();

    // 3. Sequential Issue Creation (1, 2)
    let issue_1 = Issue {
        id: "iss_1".into(),
        repo_id: repo_id.into(),
        issue_number: 0, // Auto-sequenced
        author_id: author.id.clone(),
        title: "Fix memory leak in network daemon".into(),
        body: "Daemon does not release buffer on TLS close".into(),
        status: "OPEN".into(),
        milestone_id: Some(ms.id.clone()),
        closed_by_user_id: None,
        closed_at_nanos: None,
        created_at_nanos: 200,
        updated_at_nanos: 200,
    };
    let created_1 = db.create_issue(issue_1).await.unwrap();
    assert_eq!(created_1.issue_number, 1);

    let issue_2 = Issue {
        id: "iss_2".into(),
        repo_id: repo_id.into(),
        issue_number: 0,
        author_id: author.id.clone(),
        title: "Add support for detached HEAD".into(),
        body: "Need reflog entries on detached checkout".into(),
        status: "OPEN".into(),
        milestone_id: None,
        closed_by_user_id: None,
        closed_at_nanos: None,
        created_at_nanos: 210,
        updated_at_nanos: 210,
    };
    let created_2 = db.create_issue(issue_2).await.unwrap();
    assert_eq!(created_2.issue_number, 2);

    // 4. Assignees and Labels
    db.assign_issue(&created_1.id, &assignee.id).await.unwrap();
    let assignees = db.list_issue_assignees(&created_1.id).await.unwrap();
    assert_eq!(assignees, vec![assignee.id.clone()]);

    db.add_issue_label(&created_1.id, &label.id).await.unwrap();
    let labels = db.list_issue_labels(&created_1.id).await.unwrap();
    assert_eq!(labels, vec![label.id.clone()]);

    // 5. Comments
    let comment = IssueComment {
        id: "ic_1".into(),
        issue_id: created_1.id.clone(),
        author_id: assignee.id.clone(),
        body: "I am investigating the leak in TLS handshake loop.".into(),
        created_at_nanos: 300,
        updated_at_nanos: 300,
    };
    db.create_issue_comment(comment.clone()).await.unwrap();
    let comments = db.list_issue_comments(&created_1.id).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert_eq!(comments[0].body, comment.body);

    // 6. Close Issue
    let mut to_close = created_1.clone();
    to_close.status = "CLOSED".into();
    to_close.closed_by_user_id = Some(assignee.id.clone());
    to_close.closed_at_nanos = Some(400);
    let updated = db.update_issue(to_close).await.unwrap();
    assert_eq!(updated.status, "CLOSED");

    // Filter by status
    let open_issues = db.list_issues_for_repo(repo_id, Some("OPEN")).await.unwrap();
    assert_eq!(open_issues.len(), 1);
    assert_eq!(open_issues[0].id, created_2.id);

    let closed_issues = db.list_issues_for_repo(repo_id, Some("CLOSED")).await.unwrap();
    assert_eq!(closed_issues.len(), 1);
    assert_eq!(closed_issues[0].id, created_1.id);
}
