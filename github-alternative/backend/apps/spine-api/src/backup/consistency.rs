use crate::core::error::ForgeError;
use sqlx::PgPool;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::sleep;

/// 64-bit advisory lock key identifying the cluster-wide live backup maintenance barrier.
/// Represents ASCII string "SPINEBAK" (0x5350494E_4542414B).
pub const SPINE_CLUSTER_MAINTENANCE_LOCK_ID: i64 = 0x5350_494E_4542_414B;

/// Consistency barrier for coordinating live backups and draining in-flight writes.
/// Coordinates across independent operating-system processes via PostgreSQL advisory locking
/// and a cluster maintenance state table.
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

    pub fn pool(&self) -> Option<&PgPool> {
        self.pool.as_ref()
    }

    /// Check whether the cluster or node is currently in a maintenance barrier window.
    pub fn is_maintenance_active(&self) -> bool {
        self.in_maintenance.load(Ordering::SeqCst)
    }

    /// Query the database-backed maintenance barrier state if a pool is configured.
    pub async fn is_maintenance_active_in_db(&self) -> Result<bool, ForgeError> {
        if let Some(pool) = &self.pool {
            let row: Result<Option<(bool,)>, _> =
                sqlx::query_as("SELECT is_active FROM _cluster_maintenance_barrier WHERE id = 1")
                    .fetch_optional(pool)
                    .await;
            match row {
                Ok(Some((active,))) => Ok(active),
                Ok(None) => Ok(false),
                Err(_) => Ok(self.is_maintenance_active()),
            }
        } else {
            Ok(self.is_maintenance_active())
        }
    }

    /// Check whether the cluster or node is currently in a maintenance barrier window asynchronously.
    pub async fn is_maintenance_active_async(&self) -> bool {
        if self.is_maintenance_active() {
            return true;
        }
        if let Ok(active) = self.is_maintenance_active_in_db().await {
            if active {
                self.in_maintenance.store(true, Ordering::SeqCst);
                return true;
            }
        }
        false
    }

    /// Acquire an in-flight write permit asynchronously.
    /// Checks local atomic flag AND queries the database-backed barrier and acquires a shared advisory lock.
    /// Fails with ServiceUnavailable if the cluster is in maintenance mode.
    pub async fn acquire_write_permit(&self) -> Result<WritePermit, ForgeError> {
        if self.is_maintenance_active() {
            return Err(ForgeError::ServiceUnavailable(
                "Cluster write pipeline is currently paused for coordinated maintenance"
                    .to_string(),
            ));
        }

        if let Some(pool) = &self.pool {
            // 1. Check if cluster maintenance table has marked maintenance active
            let active_row: Result<Option<(bool,)>, _> =
                sqlx::query_as("SELECT is_active FROM _cluster_maintenance_barrier WHERE id = 1")
                    .fetch_optional(pool)
                    .await;

            if let Ok(Some((true,))) = active_row {
                self.in_maintenance.store(true, Ordering::SeqCst);
                return Err(ForgeError::ServiceUnavailable(
                    "Cluster write pipeline is currently paused for coordinated maintenance"
                        .to_string(),
                ));
            } else if let Ok(Some((false,))) = active_row {
                if self.in_maintenance.load(Ordering::SeqCst) {
                    self.in_maintenance.store(false, Ordering::SeqCst);
                }
            }

            // 2. Acquire a dedicated connection from pool to hold the shared advisory lock
            let mut conn = pool.acquire().await.map_err(|e| {
                ForgeError::ServiceUnavailable(format!(
                    "Failed to acquire database connection for writer permit: {}",
                    e
                ))
            })?;

            // 3. Try acquiring shared session advisory lock across all cluster processes
            let lock_res: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock_shared($1)")
                .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                .fetch_one(&mut *conn)
                .await
                .map_err(|e| {
                    ForgeError::ServiceUnavailable(format!(
                        "Failed to query advisory lock for writer permit: {}",
                        e
                    ))
                })?;

            if !lock_res.0 {
                self.in_maintenance.store(true, Ordering::SeqCst);
                return Err(ForgeError::ServiceUnavailable(
                    "Cluster write pipeline is currently paused for coordinated maintenance"
                        .to_string(),
                ));
            }

            self.in_flight_writes.fetch_add(1, Ordering::SeqCst);

            // Channel for releasing the shared lock cleanly when permit is dropped
            let (drop_tx, drop_rx) = oneshot::channel::<()>();

            tokio::spawn(async move {
                let _ = drop_rx.await;
                let unlock_res = sqlx::query("SELECT pg_advisory_unlock_shared($1)")
                    .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                    .execute(&mut *conn)
                    .await;

                if unlock_res.is_err() {
                    // If unlock fails, close connection so it NEVER returns to pool while locked
                    let _ = conn.close().await;
                }
            });

            Ok(WritePermit {
                _drop_guard: Some(drop_tx),
                counter: self.in_flight_writes.clone(),
            })
        } else {
            // In-memory fallback
            self.in_flight_writes.fetch_add(1, Ordering::SeqCst);
            Ok(WritePermit {
                _drop_guard: None,
                counter: self.in_flight_writes.clone(),
            })
        }
    }

    /// Synchronous version for non-async contexts
    pub fn acquire_write_permit_sync(&self) -> Result<WritePermit, ForgeError> {
        if self.is_maintenance_active() {
            return Err(ForgeError::ServiceUnavailable(
                "Cluster write pipeline is currently paused for coordinated maintenance"
                    .to_string(),
            ));
        }
        self.in_flight_writes.fetch_add(1, Ordering::SeqCst);
        Ok(WritePermit {
            _drop_guard: None,
            counter: self.in_flight_writes.clone(),
        })
    }

    /// Enter maintenance mode and drain in-flight operations across all cluster processes.
    pub async fn enter_maintenance(
        &self,
        drain_timeout: Duration,
    ) -> Result<MaintenanceGuard, ForgeError> {
        self.in_maintenance.store(true, Ordering::SeqCst);
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let mut guard_conn = None;

        if let Some(pool) = &self.pool {
            // 1. Initialize maintenance table and mark barrier active so all processes reject new writers
            sqlx::query(
                r#"
                CREATE TABLE IF NOT EXISTS _cluster_maintenance_barrier (
                    id INT PRIMARY KEY DEFAULT 1,
                    is_active BOOLEAN NOT NULL DEFAULT FALSE,
                    activated_at_nanos BIGINT NOT NULL DEFAULT 0,
                    activated_by VARCHAR(255) NOT NULL DEFAULT '',
                    reason VARCHAR(255) NOT NULL DEFAULT ''
                )
                "#,
            )
            .execute(pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to create cluster maintenance barrier table in database: {}",
                    e
                ))
            })?;

            sqlx::query(
                r#"
                INSERT INTO _cluster_maintenance_barrier (id, is_active, activated_at_nanos, activated_by, reason)
                VALUES (1, TRUE, $1, 'spine-backup', 'Coordinated live cluster backup')
                ON CONFLICT (id) DO UPDATE SET 
                    is_active = TRUE,
                    activated_at_nanos = $1,
                    activated_by = 'spine-backup',
                    reason = 'Coordinated live cluster backup'
                "#,
            )
            .bind(now_nanos)
            .execute(pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to activate cluster maintenance barrier in database: {}",
                    e
                ))
            })?;

            // 2. Check out dedicated connection to acquire exclusive advisory lock
            let mut conn = pool.acquire().await.map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to acquire connection for maintenance lock: {}",
                    e
                ))
            })?;

            // 3. Acquire exclusive advisory lock, waiting for existing in-flight shared locks to drain
            let start = std::time::Instant::now();
            let mut acquired = false;
            while start.elapsed() < drain_timeout {
                let lock_res: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock($1)")
                    .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                    .fetch_one(&mut *conn)
                    .await
                    .map_err(|e| {
                        ForgeError::Internal(format!(
                            "Failed to query exclusive maintenance lock: {}",
                            e
                        ))
                    })?;

                if lock_res.0 {
                    acquired = true;
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }

            if !acquired {
                let _ = sqlx::query(
                    "UPDATE _cluster_maintenance_barrier SET is_active = FALSE WHERE id = 1",
                )
                .execute(pool)
                .await;
                self.in_maintenance.store(false, Ordering::SeqCst);
                return Err(ForgeError::Internal(format!(
                    "Timed out after {:?} waiting for in-flight writers to drain across cluster",
                    drain_timeout
                )));
            }

            guard_conn = Some(conn);
        }

        tracing::info!(
            "Maintenance consistency barrier activated. Draining in-flight writes for {:?}...",
            drain_timeout
        );

        let start = std::time::Instant::now();
        while self.in_flight_writes.load(Ordering::SeqCst) > 0 && start.elapsed() < drain_timeout {
            sleep(Duration::from_millis(20)).await;
        }

        Ok(MaintenanceGuard {
            barrier: self.clone(),
            _conn: guard_conn,
        })
    }
}

pub struct WritePermit {
    _drop_guard: Option<oneshot::Sender<()>>,
    counter: Arc<AtomicUsize>,
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::SeqCst);
    }
}

pub struct MaintenanceGuard {
    barrier: ConsistencyBarrier,
    _conn: Option<sqlx::pool::PoolConnection<sqlx::Postgres>>,
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        self.barrier.in_maintenance.store(false, Ordering::SeqCst);
        let conn_opt = self._conn.take();
        if let Some(pool) = self.barrier.pool.clone() {
            tokio::spawn(async move {
                let _ = sqlx::query(
                    "UPDATE _cluster_maintenance_barrier SET is_active = FALSE WHERE id = 1",
                )
                .execute(&pool)
                .await;

                if let Some(mut conn) = conn_opt {
                    let unlock_res = sqlx::query("SELECT pg_advisory_unlock($1)")
                        .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                        .execute(&mut *conn)
                        .await;
                    if unlock_res.is_err() {
                        let _ = conn.close().await;
                    }
                }
            });
        }
        tracing::info!("Maintenance consistency barrier deactivated.");
    }
}
