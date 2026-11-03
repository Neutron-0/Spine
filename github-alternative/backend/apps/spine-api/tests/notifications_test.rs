use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;

#[tokio::test]
async fn test_notifications_lifecycle() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let user_id = "usr_target";

    // 1. Create two notifications
    let notif_1 = UserNotification {
        id: "notif_1".into(),
        user_id: user_id.into(),
        reason: "PR_REVIEW".into(),
        resource_type: "pull_request".into(),
        resource_id: "pr_1".into(),
        title: "Review requested".into(),
        message: "Alice requested your review on PR #1".into(),
        is_read: false,
        created_at_nanos: 100,
    };
    let notif_2 = UserNotification {
        id: "notif_2".into(),
        user_id: user_id.into(),
        reason: "ISSUE_ASSIGNED".into(),
        resource_type: "issue".into(),
        resource_id: "iss_42".into(),
        title: "Assigned to issue".into(),
        message: "You were assigned to issue #42".into(),
        is_read: false,
        created_at_nanos: 200,
    };
    db.create_notification(notif_1.clone()).await.unwrap();
    db.create_notification(notif_2.clone()).await.unwrap();

    // 2. List unread
    let unread = db.list_user_notifications(user_id, true).await.unwrap();
    assert_eq!(unread.len(), 2);

    // 3. Mark one read
    db.mark_notification_read(&notif_1.id, user_id).await.unwrap();
    let unread_after = db.list_user_notifications(user_id, true).await.unwrap();
    assert_eq!(unread_after.len(), 1);
    assert_eq!(unread_after[0].id, notif_2.id);

    // 4. Mark all read
    db.mark_all_notifications_read(user_id).await.unwrap();
    let unread_final = db.list_user_notifications(user_id, true).await.unwrap();
    assert_eq!(unread_final.len(), 0);

    let all_final = db.list_user_notifications(user_id, false).await.unwrap();
    assert_eq!(all_final.len(), 2);
    assert!(all_final.iter().all(|n| n.is_read));
}
