use std::collections::HashMap;
use std::sync::RwLock;

pub struct RateLimiter {
    // Key -> List of timestamp nanos
    attempts: RwLock<HashMap<String, Vec<i64>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            attempts: RwLock::new(HashMap::new()),
        }
    }

    /// Returns `true` if operation is allowed under `max_attempts` within `window_secs`.
    pub fn check_and_record(&self, key: &str, max_attempts: usize, window_secs: i64) -> bool {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let window_nanos = window_secs * 1_000_000_000;
        let cutoff = now - window_nanos;

        let mut lock = self.attempts.write().unwrap();
        let list = lock.entry(key.to_string()).or_default();
        list.retain(|&ts| ts > cutoff);

        if list.len() >= max_attempts {
            false
        } else {
            list.push(now);
            true
        }
    }
}

impl Default for RateLimiter {
    fn default() -> Self {
        Self::new()
    }
}
