use chrono::Utc;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::time::sleep;

use spine_api::core::types::*;
use spine_api::db::{Database, InMemoryDatabase};
use spine_api::webhook::{SsrfValidator, WebhookOutboxWorker};
use spine_api::workflow::WorkflowScheduler;

#[tokio::test]
async fn test_webhook_worker_lifecycle_and_processing() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let ssrf = SsrfValidator::new(true); // allow loopback for test
    let worker = Arc::new(WebhookOutboxWorker::new(Arc::clone(&db), ssrf));

    // Seed outbox item
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let item = WebhookOutboxItem {
        id: "outbox_1".to_string(),
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

    // Verify item is pending
    assert_eq!(
        db.count_pending_webhook_outbox(now + 1000).await.unwrap(),
        1
    );

    // Spawn worker
    let worker_clone = Arc::clone(&worker);
    let handle = tokio::spawn(async move {
        worker_clone.run().await;
    });

    // Wait briefly for worker to process (no subscribers, so item transitions to DELIVERED)
    let mut processed = false;
    for _ in 0..20 {
        sleep(Duration::from_millis(50)).await;
        if db.count_pending_webhook_outbox(now + 1000).await.unwrap() == 0 {
            processed = true;
            break;
        }
    }

    assert!(
        processed,
        "WebhookOutboxWorker daemon loop failed to process pending outbox item"
    );

    // Verify shutdown stops the loop
    worker.shutdown();
    tokio::time::timeout(Duration::from_millis(1000), handle)
        .await
        .expect("Worker did not shut down within timeout")
        .unwrap();
}

#[tokio::test]
async fn test_webhook_worker_delivers_over_https() {
    // 1. Generate self-signed certificate for localhost
    let (cert_der, key_der) =
        spine_net::tls::generate_self_signed_cert_der(&["localhost", "127.0.0.1"]).unwrap();
    let server_tls_config =
        Arc::new(spine_net::tls::build_server_tls_config(vec![cert_der.clone()], key_der).unwrap());

    // 2. Start mock HTTPS server on loopback
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();

    let acceptor = spine_net::tls::tokio_rustls::TlsAcceptor::from(server_tls_config);
    let server_handle = tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            if let Ok(mut tls_stream) = acceptor.accept(stream).await {
                let mut buf = [0u8; 1024];
                let n = tls_stream.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                assert!(req.contains("POST /webhook HTTP/1.1"));
                assert!(req.contains("X-Spine-Event: push"));

                let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nOK";
                let _ = tls_stream.write_all(resp.as_bytes()).await;
                let _ = tls_stream.flush().await;
            }
        }
    });

    // 3. Setup DB with subscription pointing to HTTPS target
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let sub = WebhookSubscription {
        id: "sub_tls_1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo_tls".to_string(),
        target_url: format!("https://127.0.0.1:{}/webhook", port),
        secret_ciphertext: "test_secret".to_string(),
        secret_hash: "test_hash".to_string(),
        events: "push".to_string(),
        is_active: true,
        created_at_nanos: 0,
        updated_at_nanos: 0,
    };
    db.create_webhook_subscription(sub).await.unwrap();

    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let item = WebhookOutboxItem {
        id: "outbox_tls_1".to_string(),
        owner_type: "REPO".to_string(),
        owner_id: "repo_tls".to_string(),
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

    // 4. Create worker configured with custom root cert (our self-signed cert)
    let ssrf = SsrfValidator::new(true);
    let worker = WebhookOutboxWorker::new(Arc::clone(&db), ssrf).with_root_certs(vec![cert_der]);

    // 5. Process batch
    let processed = worker.process_batch(10).await.unwrap();
    assert_eq!(processed, 1);

    // 6. Verify delivery log
    let deliveries = db.list_webhook_deliveries("sub_tls_1", 10).await.unwrap();
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].status, "SUCCESS");
    assert_eq!(deliveries[0].response_status, Some(200));

    let _ = server_handle.await;
}

#[tokio::test]
async fn test_workflow_scheduler_lifecycle() {
    let db: Arc<dyn Database> = Arc::new(InMemoryDatabase::new());
    let scheduler = Arc::new(WorkflowScheduler::new(Arc::clone(&db), None));

    // Spawn scheduler daemon loop
    let sched_clone = Arc::clone(&scheduler);
    let handle = tokio::spawn(async move {
        sched_clone.run().await;
    });

    sleep(Duration::from_millis(100)).await;

    // Verify shutdown stops the loop cleanly
    scheduler.shutdown();
    tokio::time::timeout(Duration::from_millis(1000), handle)
        .await
        .expect("WorkflowScheduler did not shut down within timeout")
        .unwrap();
}
