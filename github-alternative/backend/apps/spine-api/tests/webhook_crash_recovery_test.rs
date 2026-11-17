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

#[tokio::test]
async fn test_webhook_outbox_lease_renewal_prevents_duplicate_reclaim_during_long_processing() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

    for i in 1..=3 {
        let item = WebhookOutboxItem {
            id: format!("outbox_batch_{}", i),
            owner_type: "REPO".to_string(),
            owner_id: "repo_1".to_string(),
            event_type: "push".to_string(),
            payload: serde_json::json!({"item": i}),
            status: "PENDING".to_string(),
            attempts: 0,
            max_attempts: 3,
            last_error: None,
            next_attempt_at_nanos: now,
            created_at_nanos: now,
        };
        db.enqueue_webhook_outbox(item).await.unwrap();
    }

    // Worker 1 claims a batch of 3 items
    let claimed = db.claim_pending_webhook_outbox(3, now).await.unwrap();
    assert_eq!(claimed.len(), 3);

    // Worker 1 is processing item #1 and calls extend_webhook_outbox_lease as worker.rs does
    db.extend_webhook_outbox_lease("outbox_batch_1", now + 120 * 1_000_000_000)
        .await
        .unwrap();

    let seventy_secs_later = now + 70 * 1_000_000_000;

    // Check that Worker 2 calling claim_pending_webhook_outbox at t = 70s does NOT reclaim any items
    // because item 1 was actively extended, and items 2 & 3 have buffer leases (90s and 120s)
    let worker_2_claim = db.claim_pending_webhook_outbox(3, seventy_secs_later).await.unwrap();
    assert_eq!(
        worker_2_claim.len(),
        0,
        "Worker 2 must not steal items from Worker 1's active batch at t=70s"
    );

    // Worker 1 extends item #2's lease before delivering it to an external server with many subscribers
    let new_lease = seventy_secs_later + 180 * 1_000_000_000;
    let extended = db
        .extend_webhook_outbox_lease("outbox_batch_2", new_lease)
        .await
        .unwrap();
    assert!(
        extended,
        "Lease extension for active processing item must succeed"
    );

    // At t = 150s, batch initial lease (120s) has expired, but item #2 lease was renewed until t=250s
    let t_150 = now + 150 * 1_000_000_000;
    // Worker 1 marks item #1 DELIVERED
    db.update_webhook_outbox_status("outbox_batch_1", "DELIVERED", None, 0)
        .await
        .unwrap();

    // Worker 2 tries to claim at t = 150s:
    // Item 1 is DELIVERED.
    // Item 2 is PROCESSING with lease until t=250s (not expired).
    // Item 3 had no extension and its 120s lease expired at t=120s (Worker 1 simulated crash before touching item 3).
    let worker_2_reclaim = db.claim_pending_webhook_outbox(3, t_150).await.unwrap();
    assert_eq!(
        worker_2_reclaim.len(),
        1,
        "Only crashed item 3 should be reclaimed"
    );
    assert_eq!(worker_2_reclaim[0].id, "outbox_batch_3");
}
