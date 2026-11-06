use crate::core::error::ForgeError;
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// Consistency barrier for coordinating live backups and draining in-flight writes.
/// Supports both in-process permit tracking and cross-process database-backed state.
#[derive(Clone, Default)]
pub struct ConsistencyBarrier {
    in_maintenance: Arc<AtomicBool>,
    in_flight_writes: Arc<AtomicUsize>,
    pool: Option<PgPool>,
}

impl ConsistencyBarrier {
    pub fn new() -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            pool: None,
        }
    }

    pub fn with_pool(pool: PgPool) -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            pool: Some(pool),
        }
    }

    /// Check whether the cluster or node is currently in a maintenance barrier window.
    pub fn is_maintenance_active(&self) -> bool {
        self.in_maintenance.load(Ordering::SeqCst)
    }

    /// Query the database-backed maintenance barrier state if a pool is configured.
    pub async fn is_maintenance_active_in_db(&self) -> Result<bool, ForgeError> {
        if let Some(pool) = &self.pool {
            let row: Result<(bool,), _> =
                sqlx::query_as("SELECT is_active FROM _cluster_maintenance_barrier WHERE id = 1")
                    .fetch_one(pool)
                    .await;
            match row {
                Ok((active,)) => Ok(active),
                Err(_) => Ok(self.is_maintenance_active()),
            }
        } else {
            Ok(self.is_maintenance_active())
        }
    }

    /// Acquire an in-flight write permit.
    /// Fails with ServiceUnavailable if the cluster is in maintenance mode.
    pub fn acquire_write_permit(&self) -> Result<WritePermit, ForgeError> {
        if self.is_maintenance_active() {
            return Err(ForgeError::ServiceUnavailable(
                "Cluster write pipeline is currently paused for coordinated maintenance"
                    .to_string(),
            ));
        }
        self.in_flight_writes.fetch_add(1, Ordering::SeqCst);
        Ok(WritePermit {
            counter: self.in_flight_writes.clone(),
        })
    }

    /// Enter maintenance mode and drain in-flight operations.
    pub async fn enter_maintenance(&self, drain_timeout: Duration) -> MaintenanceGuard {
        self.in_maintenance.store(true, Ordering::SeqCst);
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        if let Some(pool) = &self.pool {
            let _ = sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS _cluster_maintenance_barrier (
                    id INT PRIMARY KEY DEFAULT 1,
                    is_active BOOLEAN NOT NULL DEFAULT FALSE,
                    activated_at_nanos BIGINT NOT NULL DEFAULT 0,
                    activated_by VARCHAR(255) NOT NULL DEFAULT '',
                    reason VARCHAR(255) NOT NULL DEFAULT ''
                );
                INSERT INTO _cluster_maintenance_barrier (id, is_active, activated_at_nanos, activated_by, reason)
                VALUES (1, TRUE, $1, 'spine-backup', 'Coordinated live cluster backup')
                ON CONFLICT (id) DO UPDATE SET 
                    is_active = TRUE,
                    activated_at_nanos = $1,
                    activated_by = 'spine-backup',
                    reason = 'Coordinated live cluster backup';
                "#,
            )
            .bind(now_nanos)
            .execute(pool)
            .await;
        }

        tracing::info!(
            "Maintenance consistency barrier activated. Draining in-flight writes for {:?}...",
            drain_timeout
        );

        let start = std::time::Instant::now();
        while self.in_flight_writes.load(Ordering::SeqCst) > 0 && start.elapsed() < drain_timeout {
            sleep(Duration::from_millis(20)).await;
        }

        MaintenanceGuard {
            barrier: self.clone(),
        }
    }
}

pub struct WritePermit {
    counter: Arc<AtomicUsize>,
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct MaintenanceGuard {
    barrier: ConsistencyBarrier,
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        self.barrier.in_maintenance.store(false, Ordering::SeqCst);
        if let Some(pool) = self.barrier.pool.clone() {
            tokio::spawn(async move {
                let _ = sqlx::query(
                    "UPDATE _cluster_maintenance_barrier SET is_active = FALSE WHERE id = 1",
                )
                .execute(&pool)
                .await;
            });
        }
        tracing::info!("Maintenance consistency barrier deactivated.");
    }
}
