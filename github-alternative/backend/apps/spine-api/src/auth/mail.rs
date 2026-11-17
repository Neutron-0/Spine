use async_trait::async_trait;
use rand::RngCore;
use std::sync::RwLock;

#[derive(Clone, Debug)]
pub struct SentEmail {
    pub recipient: String,
    pub subject: String,
    pub body: String,
    pub timestamp_nanos: i64,
}

#[async_trait]
pub trait MailDelivery: Send + Sync {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()>;
}

pub struct DevMailCatcher {
    sent_emails: RwLock<Vec<SentEmail>>,
}

impl DevMailCatcher {
    pub fn new() -> Self {
        Self {
            sent_emails: RwLock::new(Vec::new()),
        }
    }

    pub fn get_sent_emails(&self) -> Vec<SentEmail> {
        self.sent_emails.read().unwrap().clone()
    }

    pub fn last_email_for(&self, email: &str) -> Option<SentEmail> {
        self.sent_emails
            .read()
            .unwrap()
            .iter()
            .rev()
            .find(|e| e.recipient == email)
            .cloned()
    }
}

impl Default for DevMailCatcher {
    fn default() -> Self {
        Self::new()
    }
}

pub const MAX_CACHED_EMAILS: usize = 1000;

#[async_trait]
impl MailDelivery for DevMailCatcher {
    async fn send_email(&self, to: &str, subject: &str, body: &str) -> anyhow::Result<()> {
        let sent = SentEmail {
            recipient: to.to_string(),
            subject: subject.to_string(),
            body: body.to_string(),
            timestamp_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };
        tracing::info!(recipient = %to, subject = %subject, "Authentication email queued for delivery");
        let mut list = self.sent_emails.write().unwrap();
        if list.len() >= MAX_CACHED_EMAILS {
            list.remove(0);
        }
        list.push(sent);
        Ok(())
    }
}

pub fn generate_verification_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("sp_vtok_{}", hex::encode(bytes))
}

pub fn generate_reset_token() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    format!("sp_rtok_{}", hex::encode(bytes))
}

pub fn hash_token(token: &str) -> String {
    blake3::hash(token.as_bytes()).to_hex().to_string()
}
