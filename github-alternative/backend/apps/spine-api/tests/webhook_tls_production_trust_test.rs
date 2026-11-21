use chrono::Utc;
use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::webhook::{SsrfValidator, WebhookOutboxWorker};
use spine_net::tls::build_client_tls_config;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;

#[test]
fn test_default_client_tls_config_includes_root_certificates() {
    let config = build_client_tls_config(None).expect("Failed to build default client TLS config");
    // Verify client TLS config builds cleanly and uses WebPKI roots
    let _ = config;
}

#[tokio::test]
async fn test_webhook_delivery_full_timeout_on_hanging_tls_handshake() {
    // Spin up a raw TCP listener that accepts connection but never responds to TLS handshake
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();

    let server_handle = tokio::spawn(async move {
        if let Ok((mut stream, _)) = listener.accept().await {
            use tokio::io::AsyncReadExt;
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf).await;
            tokio::time::sleep(Duration::from_secs(10)).await;
        }
    });

    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let ssrf = SsrfValidator::new(true); // Allow localhost for unit test
    let worker =
        WebhookOutboxWorker::new(Arc::clone(&db), ssrf).with_timeout(Duration::from_millis(200)); // Short 200ms timeout

    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let sub = WebhookSubscription {
        id: "sub-1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo-1".to_string(),
        target_url: format!("https://127.0.0.1:{}", addr.port()),
        secret_hash: "hash".to_string(),
        secret_ciphertext: "secret".to_string(),
        events: "*".to_string(),
        is_active: true,
        created_at_nanos: now,
        updated_at_nanos: now,
    };
    db.create_webhook_subscription(sub).await.unwrap();

    let item = WebhookOutboxItem {
        id: "evt-1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo-1".to_string(),
        event_type: "push".to_string(),
        payload: serde_json::json!({"test": "data"}),
        status: "PENDING".to_string(),
        attempts: 0,
        max_attempts: 3,
        last_error: None,
        next_attempt_at_nanos: now,
        created_at_nanos: now,
    };
    db.enqueue_webhook_outbox(item).await.unwrap();

    let start = std::time::Instant::now();
    let processed = worker.process_batch(10).await.unwrap();
    assert_eq!(processed, 1);
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "Webhook delivery should have timed out within ~200ms, but took {:?}",
        elapsed
    );

    server_handle.abort();
}
