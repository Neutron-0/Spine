use chrono::Utc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::timeout;
use uuid::Uuid;

use spine_net::tls::SpineStream;

use super::service::WebhookService;
use super::ssrf::SsrfValidator;
use crate::core::error::ForgeError;
use crate::core::types::{WebhookDelivery, WebhookOutboxItem, WebhookSubscription};
use crate::db::Database;

pub struct WebhookOutboxWorker {
    db: Arc<dyn Database>,
    ssrf: SsrfValidator,
    timeout_duration: std::time::Duration,
    consistency_barrier: Option<Arc<crate::backup::ConsistencyBarrier>>,
    running: Arc<AtomicBool>,
    root_certs: Option<Vec<Vec<u8>>>,
}

impl WebhookOutboxWorker {
    pub fn new(db: Arc<dyn Database>, ssrf: SsrfValidator) -> Self {
        Self {
            db,
            ssrf,
            timeout_duration: std::time::Duration::from_secs(10),
            consistency_barrier: None,
            running: Arc::new(AtomicBool::new(true)),
            root_certs: None,
        }
    }

    pub fn with_barrier(mut self, barrier: Arc<crate::backup::ConsistencyBarrier>) -> Self {
        self.consistency_barrier = Some(barrier);
        self
    }

    pub fn with_root_certs(mut self, certs: Vec<Vec<u8>>) -> Self {
        self.root_certs = Some(certs);
        self
    }

    pub fn with_timeout(mut self, timeout: std::time::Duration) -> Self {
        self.timeout_duration = timeout;
        self
    }

    pub fn shutdown(&self) {
        self.running.store(false, Ordering::SeqCst);
    }

    /// Background worker loop.
    pub async fn run(&self) {
        while self.running.load(Ordering::SeqCst) {
            match self.process_batch(10).await {
                Ok(count) if count > 0 => {
                    // Items processed; check again immediately
                    continue;
                }
                Ok(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(e) => {
                    tracing::warn!("Webhook outbox worker error: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Process a batch of scheduled outbox webhook events.
    pub async fn process_batch(&self, limit: i64) -> Result<usize, ForgeError> {
        let _permit = if let Some(barrier) = &self.consistency_barrier {
            match barrier.acquire_write_permit().await {
                Ok(p) => Some(p),
                Err(e) => {
                    tracing::warn!(
                        "WebhookOutboxWorker paused during cluster maintenance: {}",
                        e
                    );
                    return Ok(0);
                }
            }
        } else {
            None
        };

        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let pending = self
            .db
            .claim_pending_webhook_outbox(limit, now_nanos)
            .await?;
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

            // Actively refresh/extend the lease for this specific event before and during delivery
            let per_sub_timeout = self.timeout_duration.as_secs().max(5);
            let needed_secs = (per_sub_timeout * (matching_subs.len() as u64) + 60) as i64;
            let current_now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let extended = self
                .db
                .extend_webhook_outbox_lease(&event.id, current_now + needed_secs * 1_000_000_000)
                .await?;
            if !extended {
                // Lost lease or item completed elsewhere; skip to prevent duplicate delivery!
                tracing::warn!("Webhook outbox item {} lost lease; skipping delivery", event.id);
                continue;
            }

            let mut any_failed = false;
            let mut last_error = None;

            for sub in matching_subs {
                let current_now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
                let _ = self
                    .db
                    .extend_webhook_outbox_lease(
                        &event.id,
                        current_now + (self.timeout_duration.as_secs().max(5) + 30) as i64 * 1_000_000_000,
                    )
                    .await;

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
                            status: if is_success {
                                "SUCCESS".to_string()
                            } else {
                                "FAILED".to_string()
                            },
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
                self.handle_failure(
                    event,
                    last_error.unwrap_or_else(|| "Unknown failure".to_string()),
                )
                .await?;
            }
        }

        Ok(count)
    }

    async fn handle_failure(
        &self,
        event: WebhookOutboxItem,
        err_msg: String,
    ) -> Result<(), ForgeError> {
        let attempts = event.attempts + 1;
        if attempts >= event.max_attempts {
            self.db
                .update_webhook_outbox_status(&event.id, "FAILED", Some(&err_msg), 0)
                .await?;
        } else {
            // Exponential backoff: base (1s) * 2^(attempts-1) in nanoseconds
            let backoff_secs = 1i64 << (attempts - 1);
            let next_scheduled_nanos =
                Utc::now().timestamp_nanos_opt().unwrap_or(0) + (backoff_secs * 1_000_000_000);
            self.db
                .update_webhook_outbox_status(
                    &event.id,
                    "PENDING",
                    Some(&err_msg),
                    next_scheduled_nanos,
                )
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

        timeout(self.timeout_duration, async {
            let host = url.host_str().unwrap_or("localhost");
            let path = if url.path().is_empty() {
                "/"
            } else {
                url.path()
            };
            let query = url.query().map(|q| format!("?{}", q)).unwrap_or_default();
            let target_resource = format!("{}{}", path, query);

            let payload_str = serde_json::to_string(&event.payload)?;
            let signature =
                WebhookService::sign_payload(&sub.secret_ciphertext, payload_str.as_bytes());

            // Connect
            let tcp_stream = TcpStream::connect(socket_addr).await?;
            let mut stream = if url.scheme() == "https" {
                let tls_config = Arc::new(
                    spine_net::tls::build_client_tls_config(self.root_certs.clone())
                        .map_err(|e| anyhow::anyhow!("TLS config error: {}", e))?,
                );
                SpineStream::upgrade_client(tcp_stream, host, tls_config)
                    .await
                    .map_err(|e| anyhow::anyhow!("TLS handshake error: {}", e))?
            } else {
                SpineStream::Plain(tcp_stream)
            };

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
            while let Ok(n) = stream.read(&mut chunk).await {
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
            let status_line = lines
                .next()
                .ok_or_else(|| anyhow::anyhow!("Empty HTTP response"))?;

            let parts: Vec<&str> = status_line.split_whitespace().collect();
            if parts.len() < 2 {
                return Err(anyhow::anyhow!(
                    "Malformed HTTP status line: {}",
                    status_line
                ));
            }
            let status_code: i32 = parts[1]
                .parse()
                .map_err(|_| anyhow::anyhow!("Invalid status code"))?;

            let body = if let Some(idx) = resp_str.find("\r\n\r\n") {
                resp_str[idx + 4..].to_string()
            } else if let Some(idx) = resp_str.find("\n\n") {
                resp_str[idx + 2..].to_string()
            } else {
                String::new()
            };

            Ok((status_code, body))
        })
        .await
        .map_err(|_| anyhow::anyhow!("Webhook delivery timed out after {:?}", self.timeout_duration))?
    }
}
