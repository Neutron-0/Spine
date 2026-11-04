use std::sync::Arc;
use std::time::Instant;
use chrono::Utc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use crate::core::types::{WebhookDelivery, WebhookOutboxItem, WebhookSubscription};
use crate::core::error::ForgeError;
use crate::db::Database;
use super::service::WebhookService;
use super::ssrf::SsrfValidator;

pub struct WebhookOutboxWorker {
    db: Arc<dyn Database>,
    ssrf: SsrfValidator,
    timeout_duration: std::time::Duration,
}

impl WebhookOutboxWorker {
    pub fn new(db: Arc<dyn Database>, ssrf: SsrfValidator) -> Self {
        Self {
            db,
            ssrf,
            timeout_duration: std::time::Duration::from_secs(10),
        }
    }

    /// Process a batch of scheduled outbox webhook events.
    pub async fn process_batch(&self, limit: i64) -> Result<usize, ForgeError> {
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let pending = self.db.claim_pending_webhook_outbox(limit, now_nanos).await?;
        let count = pending.len();

        for event in pending {
            let subs = self
                .db
                .list_webhook_subscriptions(&event.owner_type, &event.owner_id)
                .await?;

            let matching_subs: Vec<WebhookSubscription> = subs
                .into_iter()
                .filter(|s| {
                    s.is_active
                        && s.events
                            .split(',')
                            .map(|e| e.trim())
                            .any(|e| e == "*" || e == event.event_type)
                })
                .collect();

            if matching_subs.is_empty() {
                // No active subscribers for this event
                self.db
                    .update_webhook_outbox_status(&event.id, "DELIVERED", None, 0)
                    .await?;
                continue;
            }

            let mut any_failed = false;
            let mut last_error = None;

            for sub in matching_subs {
                let start = Instant::now();
                let delivery_res = self.deliver_to_subscription(&sub, &event).await;
                let duration_ms = start.elapsed().as_millis() as i64;
                let delivered_at = Utc::now().timestamp_nanos_opt().unwrap_or(0);

                match delivery_res {
                    Ok((status_code, resp_body)) => {
                        let is_success = (200..300).contains(&status_code);
                        let delivery_log = WebhookDelivery {
                            id: Uuid::new_v4().to_string(),
                            subscription_id: sub.id.clone(),
                            event_type: event.event_type.clone(),
                            payload: event.payload.clone(),
                            response_status: Some(status_code),
                            response_body: Some(resp_body),
                            execution_duration_ms: duration_ms,
                            attempt: event.attempts + 1,
                            status: if is_success { "SUCCESS".to_string() } else { "FAILED".to_string() },
                            delivered_at_nanos: delivered_at,
                        };
                        let _ = self.db.create_webhook_delivery(delivery_log).await;

                        if !is_success {
                            any_failed = true;
                            last_error = Some(format!("HTTP status {}", status_code));
                        }
                    }
                    Err(err) => {
                        any_failed = true;
                        let err_msg = err.to_string();
                        let delivery_log = WebhookDelivery {
                            id: Uuid::new_v4().to_string(),
                            subscription_id: sub.id.clone(),
                            event_type: event.event_type.clone(),
                            payload: event.payload.clone(),
                            response_status: None,
                            response_body: None,
                            execution_duration_ms: duration_ms,
                            attempt: event.attempts + 1,
                            status: "FAILED".to_string(),
                            delivered_at_nanos: delivered_at,
                        };
                        let _ = self.db.create_webhook_delivery(delivery_log).await;
                        last_error = Some(err_msg);
                    }
                }
            }

            if !any_failed {
                self.db
                    .update_webhook_outbox_status(&event.id, "DELIVERED", None, 0)
                    .await?;
            } else {
                self.handle_failure(event, last_error.unwrap_or_else(|| "Unknown failure".to_string())).await?;
            }
        }

        Ok(count)
    }

    async fn handle_failure(&self, event: WebhookOutboxItem, err_msg: String) -> Result<(), ForgeError> {
        let attempts = event.attempts + 1;
        if attempts >= event.max_attempts {
            self.db
                .update_webhook_outbox_status(&event.id, "FAILED", Some(&err_msg), 0)
                .await?;
        } else {
            // Exponential backoff: base (1s) * 2^(attempts-1) in nanoseconds
            let backoff_secs = 1i64 << (attempts - 1);
            let next_scheduled_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0) + (backoff_secs * 1_000_000_000);
            self.db
                .update_webhook_outbox_status(&event.id, "PENDING", Some(&err_msg), next_scheduled_nanos)
                .await?;
        }
        Ok(())
    }

    /// Deliver an individual webhook payload to a single subscription via SSRF-validated TCP connection
    async fn deliver_to_subscription(
        &self,
        sub: &WebhookSubscription,
        event: &WebhookOutboxItem,
    ) -> Result<(i32, String), anyhow::Error> {
        let (url, socket_addr) = self.ssrf.validate_and_resolve(&sub.target_url).await?;

        let host = url.host_str().unwrap_or("localhost");
        let path = if url.path().is_empty() { "/" } else { url.path() };
        let query = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
        let target_resource = format!("{}{}", path, query);

        let payload_str = serde_json::to_string(&event.payload)?;
        let signature = WebhookService::sign_payload(&sub.secret_ciphertext, payload_str.as_bytes());

        // Connect with timeout
        let connect_fut = TcpStream::connect(socket_addr);
        let mut stream = timeout(self.timeout_duration, connect_fut).await??;

        // Build HTTP/1.1 POST request
        let req_str = format!(
            "POST {} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: Spine-Hookshot/1.0\r\n\
             Content-Type: application/json\r\n\
             X-Spine-Event: {}\r\n\
             X-Spine-Delivery: {}\r\n\
             X-Spine-Signature-256: {}\r\n\
             Content-Length: {}\r\n\
             Connection: close\r\n\r\n{}",
            target_resource,
            host,
            event.event_type,
            event.id,
            signature,
            payload_str.len(),
            payload_str
        );

        stream.write_all(req_str.as_bytes()).await?;
        stream.flush().await?;

        // Read response (up to 64 KiB)
        let mut response_buf = Vec::new();
        let mut chunk = [0u8; 4096];
        loop {
            let read_fut = stream.read(&mut chunk);
            let n = match timeout(self.timeout_duration, read_fut).await {
                Ok(Ok(n)) => n,
                _ => break,
            };
            if n == 0 {
                break;
            }
            response_buf.extend_from_slice(&chunk[..n]);
            if response_buf.len() > 65536 {
                break;
            }
        }

        let resp_str = String::from_utf8_lossy(&response_buf);
        let mut lines = resp_str.lines();
        let status_line = lines.next().ok_or_else(|| anyhow::anyhow!("Empty HTTP response"))?;

        let parts: Vec<&str> = status_line.split_whitespace().collect();
        if parts.len() < 2 {
            return Err(anyhow::anyhow!("Malformed HTTP status line: {}", status_line));
        }
        let status_code: i32 = parts[1].parse().map_err(|_| anyhow::anyhow!("Invalid status code"))?;

        let body = if let Some(idx) = resp_str.find("\r\n\r\n") {
            resp_str[idx + 4..].to_string()
        } else if let Some(idx) = resp_str.find("\n\n") {
            resp_str[idx + 2..].to_string()
        } else {
            String::new()
        };

        Ok((status_code, body))
    }
}
