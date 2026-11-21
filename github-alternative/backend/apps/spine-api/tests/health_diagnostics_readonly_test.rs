use axum::extract::State;
use spine_api::api::middleware::AppState;
use spine_api::api::routes_health::health_diagnostics;
use spine_api::core::types::WebhookOutboxItem;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;

#[tokio::test]
async fn test_health_diagnostics_does_not_mutate_outbox_queue() {
    let db = Arc::new(InMemoryDatabase::new());
    let temp_dir = tempfile::tempdir().unwrap();

    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    // Enqueue an outbox item
    let item = WebhookOutboxItem {
        id: "outbox_test_1".to_string(),
        event_type: "push".to_string(),
        owner_type: "repository".to_string(),
        owner_id: "repo_1".to_string(),
        payload: serde_json::json!({ "ref": "refs/heads/main" }),
        attempts: 0,
        max_attempts: 5,
        status: "PENDING".to_string(),
        last_error: None,
        created_at_nanos: now,
        next_attempt_at_nanos: now,
    };
    Database::enqueue_webhook_outbox(db.as_ref(), item)
        .await
        .unwrap();

    let state = AppState::new_with_defaults(db.clone(), temp_dir.path(), None).unwrap();

    // Call health_diagnostics multiple times
    for _ in 0..3 {
        let _resp = health_diagnostics(State(state.clone())).await;
    }

    // Verify outbox item is STILL PENDING and attempts == 0
    let claimed = Database::claim_pending_webhook_outbox(db.as_ref(), 10, now)
        .await
        .unwrap();
    assert_eq!(
        claimed.len(),
        1,
        "Outbox item must not have been claimed or abandoned by health_diagnostics"
    );
    assert_eq!(claimed[0].id, "outbox_test_1");
    // Since claim_pending_webhook_outbox increments attempts from 0 to 1:
    assert_eq!(claimed[0].attempts, 1);
}
