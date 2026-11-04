use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use uuid::Uuid;

use spine_api::core::types::CreateWebhookSubscriptionRequest;
use spine_api::db::InMemoryDatabase;
use spine_api::webhook::{SsrfValidator, WebhookOutboxWorker, WebhookService};
use spine_api::Database;

#[tokio::test]
async fn test_ssrf_validator_rejects_forbidden_ranges() {
    let validator = SsrfValidator::new(false);

    // Loopback
    assert!(validator.validate_ip("127.0.0.1".parse().unwrap()).is_err());
    assert!(validator.validate_ip("::1".parse().unwrap()).is_err());

    // AWS/GCP Metadata
    assert!(validator.validate_ip("169.254.169.254".parse().unwrap()).is_err());

    // RFC 1918 Private ranges
    assert!(validator.validate_ip("10.0.0.1".parse().unwrap()).is_err());
    assert!(validator.validate_ip("172.16.0.1".parse().unwrap()).is_err());
    assert!(validator.validate_ip("192.168.1.1".parse().unwrap()).is_err());

    // Broadcast / Multicast
    assert!(validator.validate_ip("0.0.0.0".parse().unwrap()).is_err());
    assert!(validator.validate_ip("224.0.0.1".parse().unwrap()).is_err());

    // Public IP allowed
    assert!(validator.validate_ip("8.8.8.8".parse().unwrap()).is_ok());
    assert!(validator.validate_ip("1.1.1.1".parse().unwrap()).is_ok());
}

#[tokio::test]
async fn test_webhook_hmac_signing_and_constant_time_verification() {
    let secret = "super_secret_webhook_signing_key_42";
    let payload = b"{\"event\":\"push\",\"ref\":\"refs/heads/main\"}";

    let sig = WebhookService::sign_payload(secret, payload);
    assert!(sig.starts_with("sha256="));

    // Valid signature verifies true
    assert!(WebhookService::verify_signature(secret, payload, &sig));

    // Tampered payload fails
    let tampered = b"{\"event\":\"push\",\"ref\":\"refs/heads/release\"}";
    assert!(!WebhookService::verify_signature(secret, tampered, &sig));

    // Wrong secret fails
    assert!(!WebhookService::verify_signature("wrong_secret", payload, &sig));
}

#[tokio::test]
async fn test_webhook_outbox_queue_and_delivery_cycle() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    // Allow localhost for live in-memory testing
    let ssrf = SsrfValidator::new(true);
    let service = WebhookService::new(Arc::clone(&db), ssrf.clone());
    let worker = WebhookOutboxWorker::new(Arc::clone(&db), ssrf);

    // Spin up mock HTTP receiver server
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let server_task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = [0u8; 4096];
        let n = socket.read(&mut buf).await.unwrap();
        let request_str = String::from_utf8_lossy(&buf[..n]);

        // Verify required headers
        assert!(request_str.contains("User-Agent: Spine-Hookshot/1.0"));
        assert!(request_str.contains("X-Spine-Event: push"));
        assert!(request_str.contains("X-Spine-Signature-256: sha256="));

        // Reply 200 OK
        let resp = "HTTP/1.1 200 OK\r\nContent-Length: 15\r\nConnection: close\r\n\r\n{\"received\":true}";
        socket.write_all(resp.as_bytes()).await.unwrap();
        socket.flush().await.unwrap();
    });

    let target_url = format!("http://127.0.0.1:{}/webhook-endpoint", port);
    let repo_id = Uuid::new_v4().to_string();

    let sub = service
        .create_subscription(CreateWebhookSubscriptionRequest {
            owner_type: "REPO".to_string(),
            owner_id: repo_id.clone(),
            target_url,
            secret: Some("my_webhook_secret_key_abc".to_string()),
            events: vec!["push".to_string(), "pull_request".to_string()],
            is_active: Some(true),
        })
        .await
        .unwrap();

    // Dispatch a push event
    let event_payload = serde_json::json!({
        "event": "push",
        "repository_id": repo_id,
        "ref": "refs/heads/main",
        "commits": ["c0ffee1", "c0ffee2"]
    });

    let queued = service
        .dispatch_event("REPO", &repo_id, "push", &event_payload)
        .await
        .unwrap();
    assert_eq!(queued, 1, "Expected 1 matching subscription to be queued");

    // Worker delivers the outbox batch
    let processed = worker.process_batch(10).await.unwrap();
    assert_eq!(processed, 1, "Expected 1 outbox event to be processed");

    // Await mock receiver
    server_task.await.unwrap();

    // Verify delivery history was recorded
    let deliveries = db.list_webhook_deliveries(&sub.id, 10).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, "SUCCESS");
    assert_eq!(deliveries[0].response_status, Some(200));
    assert!(deliveries[0].response_body.as_ref().unwrap().contains("\"received\":true"));
}
