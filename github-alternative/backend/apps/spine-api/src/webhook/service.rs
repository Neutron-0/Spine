use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use uuid::Uuid;

use super::ssrf::SsrfValidator;
use crate::core::error::ForgeError;
use crate::core::types::{
    CreateWebhookSubscriptionRequest, WebhookOutboxItem, WebhookSubscription,
};
use crate::db::Database;

type HmacSha256 = Hmac<Sha256>;

#[derive(Debug, thiserror::Error)]
pub enum WebhookError {
    #[error("Database error: {0}")]
    Database(#[from] ForgeError),
    #[error("SSRF validation error: {0}")]
    Ssrf(#[from] super::ssrf::SsrfError),
    #[error("Subscription not found")]
    NotFound,
    #[error("Validation failed: {0}")]
    Validation(String),
}

#[derive(Clone)]
pub struct WebhookService {
    db: Arc<dyn Database>,
    ssrf: SsrfValidator,
}

impl WebhookService {
    pub fn new(db: Arc<dyn Database>, ssrf: SsrfValidator) -> Self {
        Self { db, ssrf }
    }

    /// Sign a payload body using HMAC-SHA256, returning "sha256=<hex>".
    pub fn sign_payload(secret: &str, body: &[u8]) -> String {
        let mut mac =
            HmacSha256::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");
        mac.update(body);
        let result = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(result))
    }

    /// Verify an HMAC-SHA256 signature with constant-time equality check.
    pub fn verify_signature(secret: &str, body: &[u8], signature_header: &str) -> bool {
        let expected = Self::sign_payload(secret, body);
        if signature_header.len() != expected.len() {
            return false;
        }
        signature_header
            .as_bytes()
            .ct_eq(expected.as_bytes())
            .into()
    }

    pub async fn create_subscription(
        &self,
        req: CreateWebhookSubscriptionRequest,
    ) -> Result<WebhookSubscription, WebhookError> {
        // Validate target URL against SSRF policy
        self.ssrf.validate_and_resolve(&req.target_url).await?;

        if req.events.is_empty() {
            return Err(WebhookError::Validation(
                "At least one event must be selected".into(),
            ));
        }

        let secret = req.secret.unwrap_or_else(|| {
            let mut rng_bytes = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut rng_bytes);
            hex::encode(rng_bytes)
        });

        let secret_hash = hex::encode(blake3::hash(secret.as_bytes()).as_bytes());
        let events_str = req.events.join(",");
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let sub = WebhookSubscription {
            id: Uuid::new_v4().to_string(),
            owner_type: req.owner_type,
            owner_id: req.owner_id,
            target_url: req.target_url,
            secret_hash,
            secret_ciphertext: secret,
            events: events_str,
            is_active: req.is_active.unwrap_or(true),
            created_at_nanos: now_nanos,
            updated_at_nanos: now_nanos,
        };

        let created = self.db.create_webhook_subscription(sub).await?;
        Ok(created)
    }

    pub async fn list_subscriptions(
        &self,
        owner_type: &str,
        owner_id: &str,
    ) -> Result<Vec<WebhookSubscription>, WebhookError> {
        self.db
            .list_webhook_subscriptions(owner_type, owner_id)
            .await
            .map_err(Into::into)
    }

    pub async fn get_subscription(&self, id: &str) -> Result<WebhookSubscription, WebhookError> {
        self.db
            .get_webhook_subscription(id)
            .await?
            .ok_or(WebhookError::NotFound)
    }

    pub async fn delete_subscription(&self, id: &str) -> Result<(), WebhookError> {
        self.db
            .delete_webhook_subscription(id)
            .await
            .map_err(Into::into)
    }

    /// Queue outbox events for all matching subscriptions when an event occurs
    pub async fn dispatch_event(
        &self,
        owner_type: &str,
        owner_id: &str,
        event_type: &str,
        payload: &serde_json::Value,
    ) -> Result<usize, WebhookError> {
        let subscriptions = self
            .db
            .list_webhook_subscriptions(owner_type, owner_id)
            .await?;

        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let mut queued_count = 0;

        for sub in subscriptions {
            if !sub.is_active {
                continue;
            }

            // Check if subscription matches event type
            let matches = sub
                .events
                .split(',')
                .map(|s| s.trim())
                .any(|e| e == "*" || e == event_type);

            if !matches {
                continue;
            }

            let item = WebhookOutboxItem {
                id: Uuid::new_v4().to_string(),
                event_type: event_type.to_string(),
                owner_type: owner_type.to_string(),
                owner_id: owner_id.to_string(),
                payload: payload.clone(),
                attempts: 0,
                max_attempts: 5,
                status: "PENDING".to_string(),
                last_error: None,
                created_at_nanos: now_nanos,
                next_attempt_at_nanos: now_nanos,
            };

            self.db.enqueue_webhook_outbox(item).await?;
            queued_count += 1;
        }

        Ok(queued_count)
    }
}
