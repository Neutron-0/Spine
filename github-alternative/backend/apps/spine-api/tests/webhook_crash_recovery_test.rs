use chrono::Utc;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use std::sync::Arc;

#[tokio::test]
async fn test_webhook_outbox_lease_expiry_reclaims_stranded_processing_items() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let item = WebhookOutboxItem {
        id: "outbox_stranded_1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo_1".to_string(),
        event_type: "push".to_string(),
        payload: serde_json::json!({"ref": "refs/heads/main"}),
        status: "PENDING".to_string(),
        attempts: 0,
        max_attempts: 3,
        last_error: None,
        next_attempt_at_nanos: now,
        created_at_nanos: now,
    };
    db.enqueue_webhook_outbox(item).await.unwrap();

    // 1. Initial claim should return the item and transition it to PROCESSING with a 60s lease
    let claimed = db.claim_pending_webhook_outbox(10, now).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].id, "outbox_stranded_1");

    // 2. Before lease expires (e.g. +10s), subsequent claim must NOT return the item (it is still locked by the lease)
    let ten_secs_later = now + 10 * 1_000_000_000;
    let premature_claim = db.claim_pending_webhook_outbox(10, ten_secs_later).await.unwrap();
    assert_eq!(
        premature_claim.len(),
        0,
        "Item currently within lease window must not be claimed again"
    );

    // 3. After lease expires (e.g. +65s) due to worker crash / interruption,
    // the stranded PROCESSING item MUST be reclaimed!
    let sixty_five_secs_later = now + 65 * 1_000_000_000;
    let reclaimed = db.claim_pending_webhook_outbox(10, sixty_five_secs_later).await.unwrap();
    assert_eq!(
        reclaimed.len(),
        1,
        "Stranded PROCESSING item must be reclaimed after lease expiry"
    );
    assert_eq!(reclaimed[0].id, "outbox_stranded_1");
    // Attempts should have incremented upon re-claim
    assert_eq!(reclaimed[0].attempts, 2);
}
