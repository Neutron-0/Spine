use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;

#[tokio::test]
async fn test_code_review_workflow() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let pr_id = "pr_test_1";

    // 1. Request reviewer
    db.request_pr_reviewer(pr_id, "usr_reviewer_1", "usr_author").await.unwrap();
    let requested = db.list_pr_requested_reviewers(pr_id).await.unwrap();
    assert_eq!(requested, vec!["usr_reviewer_1"]);

    // 2. Submit Review: CHANGES_REQUESTED
    let rev_1 = PullRequestReview {
        id: "rev_1".into(),
        pr_id: pr_id.into(),
        reviewer_id: "usr_reviewer_1".into(),
        commit_id: "commit_abc".into(),
        state: "CHANGES_REQUESTED".into(),
        body: "Please fix error handling on line 42".into(),
        dismissed_by_user_id: None,
        created_at_nanos: 100,
        updated_at_nanos: 100,
    };
    db.create_pr_review(rev_1.clone()).await.unwrap();

    let reviews = db.list_pr_reviews(pr_id).await.unwrap();
    assert_eq!(reviews.len(), 1);
    assert_eq!(reviews[0].state, "CHANGES_REQUESTED");

    // 3. Add inline review comment
    let comment = PullRequestReviewComment {
        id: "prc_1".into(),
        pr_id: pr_id.into(),
        review_id: Some(rev_1.id.clone()),
        author_id: "usr_reviewer_1".into(),
        file_path: "src/main.rs".into(),
        commit_id: "commit_abc".into(),
        diff_side: "RIGHT".into(),
        line_number: 42,
        body: "Unwrap without context will panic".into(),
        is_resolved: false,
        created_at_nanos: 110,
        updated_at_nanos: 110,
    };
    db.create_pr_review_comment(comment.clone()).await.unwrap();

    let comments = db.list_pr_review_comments(pr_id).await.unwrap();
    assert_eq!(comments.len(), 1);
    assert!(!comments[0].is_resolved);

    // Resolve comment
    db.resolve_pr_review_comment(&comment.id, true).await.unwrap();
    let updated_comments = db.list_pr_review_comments(pr_id).await.unwrap();
    assert!(updated_comments[0].is_resolved);

    // 4. Submit New Review: APPROVED
    let rev_2 = PullRequestReview {
        id: "rev_2".into(),
        pr_id: pr_id.into(),
        reviewer_id: "usr_reviewer_1".into(),
        commit_id: "commit_def".into(),
        state: "APPROVED".into(),
        body: "Changes addressed! Looks good.".into(),
        dismissed_by_user_id: None,
        created_at_nanos: 200,
        updated_at_nanos: 200,
    };
    db.create_pr_review(rev_2).await.unwrap();

    let reviews_now = db.list_pr_reviews(pr_id).await.unwrap();
    assert_eq!(reviews_now.len(), 2);
    assert_eq!(reviews_now[1].state, "APPROVED");

    // 5. Dismiss review test
    db.dismiss_pr_review(&rev_1.id, "usr_admin", 300).await.unwrap();
    let reviews_after_dismiss = db.list_pr_reviews(pr_id).await.unwrap();
    assert_eq!(reviews_after_dismiss[0].state, "DISMISSED");
}
