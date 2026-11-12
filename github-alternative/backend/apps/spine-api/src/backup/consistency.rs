use crate::core::error::ForgeError;
use sqlx::postgres::PgConnectOptions;
use sqlx::{Connection, PgConnection, PgPool};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;

/// 64-bit advisory lock key identifying the cluster-wide live backup maintenance barrier.
/// Represents ASCII string "SPINEBAK" (0x5350494E_4542414B).
pub const SPINE_CLUSTER_MAINTENANCE_LOCK_ID: i64 = 0x5350_494E_4542_414B;

/// Default bounded TTL for a cluster maintenance barrier lease (5 minutes).
pub const DEFAULT_MAINTENANCE_LEASE_TTL: Duration = Duration::from_secs(300);

/// Consistency barrier for coordinating live backups and draining in-flight writes.
/// Coordinates across independent operating-system processes via PostgreSQL advisory locking
/// and a cluster maintenance state table.
#[derive(Clone)]
pub struct ConsistencyBarrier {
    in_maintenance: Arc<AtomicBool>,
    in_flight_writes: Arc<AtomicUsize>,
    fencing_generation: Arc<AtomicU64>,
    pool: Option<PgPool>,
    connect_options: Option<Arc<PgConnectOptions>>,
    storage_root: Option<Arc<std::path::PathBuf>>,
}

impl Default for ConsistencyBarrier {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsistencyBarrier {
    pub fn new() -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            pool: None,
            connect_options: None,
            storage_root: None,
        }
    }

    pub fn with_pool(pool: PgPool) -> Self {
        let connect_options = pool.connect_options().clone();
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            pool: Some(pool),
            connect_options: Some(connect_options),
            storage_root: None,
        }
    }

    pub fn with_pool_and_connect_options(pool: PgPool, connect_options: PgConnectOptions) -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            pool: Some(pool),
            connect_options: Some(Arc::new(connect_options)),
            storage_root: None,
        }
    }

    pub fn is_in_maintenance(&self) -> bool {
        self.in_maintenance.load(Ordering::SeqCst)
    }

    pub fn with_pool_and_options(pool: PgPool, options: PgConnectOptions) -> Self {
        Self {
            in_maintenance: Arc::new(AtomicBool::new(false)),
            in_flight_writes: Arc::new(AtomicUsize::new(0)),
            fencing_generation: Arc::new(AtomicU64::new(1)),
            pool: Some(pool),
            connect_options: Some(Arc::new(options)),
            storage_root: None,
        }
    }

    pub fn with_storage_root(mut self, path: impl Into<std::path::PathBuf>) -> Self {
        self.storage_root = Some(Arc::new(path.into()));
        self
    }

    pub fn pool(&self) -> Option<&PgPool> {
        self.pool.as_ref()
    }

    pub fn current_fencing_generation(&self) -> u64 {
        self.fencing_generation.load(Ordering::SeqCst)
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
    /// Checks local atomic flag AND queries the database-backed barrier and acquires a shared advisory lock
    /// on a dedicated non-pooled connection.
    /// Fails with ServiceUnavailable if the cluster is in maintenance mode.
    pub async fn acquire_write_permit(&self) -> Result<WritePermit, ForgeError> {
        if self.pool.is_none() && self.is_maintenance_active() {
            return Err(ForgeError::ServiceUnavailable(
                "Cluster write pipeline is currently paused for coordinated maintenance"
                    .to_string(),
            ));
        }

        if let Some(storage_root) = &self.storage_root {
            match crate::backup::journal::RestoreJournal::load(storage_root.as_ref()) {
                Ok(Some(journal)) => {
                    if !matches!(journal.phase, crate::backup::RestorePhase::Completed) {
                        return Err(ForgeError::ServiceUnavailable(
                            format!(
                                "Cluster write pipeline is currently paused: restore session '{}' in progress (phase: {:?})",
                                journal.restore_id, journal.phase
                            ),
                        ));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(ForgeError::ServiceUnavailable(
                        format!(
                            "Cluster write pipeline is currently paused: corrupt or unreadable restore journal: {}",
                            e
                        ),
                    ));
                }
            }

            if let Some(pool) = &self.pool {
                let db_gen_res: Result<Option<(String,)>, _> =
                    sqlx::query_as("SELECT generation_token FROM _restore_generation WHERE id = 1")
                        .fetch_optional(pool)
                        .await;
                match db_gen_res {
                    Ok(Some((db_gen,))) => {
                        let s_gen_opt =
                            crate::backup::journal::RestoreJournal::read_storage_generation(
                                storage_root.as_ref(),
                            );
                        match s_gen_opt {
                            Some(ref s_gen) if s_gen == &db_gen => {}
                            Some(ref s_gen) => {
                                return Err(ForgeError::ServiceUnavailable(format!(
                                    "Cluster write pipeline is paused: authoritative database generation '{}' does not match storage generation '{}'",
                                    db_gen, s_gen
                                )));
                            }
                            None => {
                                return Err(ForgeError::ServiceUnavailable(format!(
                                    "Cluster write pipeline is paused: storage generation record missing while database has generation '{}'",
                                    db_gen
                                )));
                            }
                        }
                    }
                    Ok(None) => {
                        let table_count: Result<(i64,), _> = sqlx::query_as(
                            "SELECT count(*)::bigint FROM information_schema.tables WHERE table_schema = 'public' AND table_name NOT LIKE 'pg_%' AND table_name NOT LIKE '_spine_%'",
                        )
                        .fetch_one(pool)
                        .await;
                        match table_count {
                            Ok((count,)) if count > 0 => {
                                return Err(ForgeError::ServiceUnavailable(
                                    "Cluster write pipeline is paused: non-empty database missing generation record in _restore_generation".into(),
                                ));
                            }
                            Err(e) => {
                                return Err(ForgeError::ServiceUnavailable(format!(
                                    "Cluster write pipeline is paused: failed to query table count: {}",
                                    e
                                )));
                            }
                            _ => {}
                        }
                    }
                    Err(e) => {
                        return Err(ForgeError::ServiceUnavailable(format!(
                            "Cluster write pipeline is paused: failed to query authoritative database generation: {}",
                            e
                        )));
                    }
                }
            }
        }

        let mut permit_gen = self.fencing_generation.load(Ordering::SeqCst);

        if let Some(opts) = &self.connect_options {
            // 1. Check if cluster maintenance table has marked maintenance active and read fencing generation + lease
            if let Some(pool) = &self.pool {
                let barrier_row: Result<Option<(bool, i64, i64)>, _> = sqlx::query_as(
                    "SELECT is_active, fencing_generation, lease_expires_at_nanos FROM _cluster_maintenance_barrier WHERE id = 1",
                )
                .fetch_optional(pool)
                .await;

                match barrier_row {
                    Ok(Some((true, _, lease_exp))) => {
                        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                        if lease_exp > 0
                            && now_nanos >= lease_exp
                            && self.clear_barrier_if_unheld().await.unwrap_or(false)
                        {
                            // Stale expired maintenance lease reconciled safely because exclusive lock was not held.
                        } else {
                            self.in_maintenance.store(true, Ordering::SeqCst);
                            return Err(ForgeError::ServiceUnavailable(
                                "Cluster write pipeline is currently paused for coordinated maintenance"
                                    .to_string(),
                            ));
                        }
                    }
                    Ok(Some((false, gen, _))) => {
                        permit_gen = gen as u64;
                        self.fencing_generation.store(permit_gen, Ordering::SeqCst);
                        if self.in_maintenance.load(Ordering::SeqCst) {
                            self.in_maintenance.store(false, Ordering::SeqCst);
                        }
                    }
                    Ok(None) => {}
                    Err(_) => {
                        // Fallback if table predates lease_expires_at_nanos column
                        let legacy_row: Result<Option<(bool, i64)>, _> = sqlx::query_as(
                            "SELECT is_active, fencing_generation FROM _cluster_maintenance_barrier WHERE id = 1",
                        )
                        .fetch_optional(pool)
                        .await;
                        if let Ok(Some((true, _))) = legacy_row {
                            self.in_maintenance.store(true, Ordering::SeqCst);
                            return Err(ForgeError::ServiceUnavailable(
                                "Cluster write pipeline is currently paused for coordinated maintenance"
                                    .to_string(),
                            ));
                        } else if let Ok(Some((false, gen))) = legacy_row {
                            permit_gen = gen as u64;
                            self.fencing_generation.store(permit_gen, Ordering::SeqCst);
                            self.in_maintenance.store(false, Ordering::SeqCst);
                        }
                    }
                }
            }

            // 2. Establish a dedicated non-pooled PgConnection for session advisory locking.
            let mut conn = PgConnection::connect_with(opts.as_ref())
                .await
                .map_err(|e| {
                    ForgeError::ServiceUnavailable(format!(
                        "Failed to establish dedicated database connection for writer permit: {}",
                        e
                    ))
                })?;

            // 3. Try acquiring shared session advisory lock across all cluster processes
            let lock_res: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock_shared($1)")
                .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                .fetch_one(&mut conn)
                .await
                .map_err(|e| {
                    ForgeError::ServiceUnavailable(format!(
                        "Failed to query advisory lock for writer permit: {}",
                        e
                    ))
                })?;

            if !lock_res.0 {
                self.in_maintenance.store(true, Ordering::SeqCst);
                let _ = conn.close().await;
                return Err(ForgeError::ServiceUnavailable(
                    "Cluster write pipeline is currently paused for coordinated maintenance"
                        .to_string(),
                ));
            }

            self.in_flight_writes.fetch_add(1, Ordering::SeqCst);

            Ok(WritePermit {
                conn: Some(conn),
                counter: self.in_flight_writes.clone(),
                permit_generation: permit_gen,
                released: false,
            })
        } else {
            // In-memory fallback
            self.in_flight_writes.fetch_add(1, Ordering::SeqCst);
            Ok(WritePermit {
                conn: None,
                counter: self.in_flight_writes.clone(),
                permit_generation: permit_gen,
                released: false,
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

        if let Some(storage_root) = &self.storage_root {
            match crate::backup::journal::RestoreJournal::load(storage_root.as_ref()) {
                Ok(Some(journal)) => {
                    if !matches!(journal.phase, crate::backup::RestorePhase::Completed) {
                        return Err(ForgeError::ServiceUnavailable(
                            format!(
                                "Cluster write pipeline is currently paused: restore session '{}' in progress (phase: {:?})",
                                journal.restore_id, journal.phase
                            ),
                        ));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(ForgeError::ServiceUnavailable(
                        format!(
                            "Cluster write pipeline is currently paused: corrupt or unreadable restore journal: {}",
                            e
                        ),
                    ));
                }
            }
        }

        let permit_gen = self.fencing_generation.load(Ordering::SeqCst);
        self.in_flight_writes.fetch_add(1, Ordering::SeqCst);
        Ok(WritePermit {
            conn: None,
            counter: self.in_flight_writes.clone(),
            permit_generation: permit_gen,
            released: false,
        })
    }

    /// Clears `_cluster_maintenance_barrier.is_active` ONLY if no active backup holds the
    /// exclusive session advisory lock (`pg_try_advisory_lock_shared` succeeds).
    pub async fn clear_barrier_if_unheld(&self) -> Result<bool, ForgeError> {
        let Some(opts) = &self.connect_options else {
            return Ok(false);
        };
        let Some(pool) = &self.pool else {
            return Ok(false);
        };
        let mut conn = PgConnection::connect_with(opts.as_ref())
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to connect for maintenance lock probe: {}",
                    e
                ))
            })?;
        let shared_ok: (bool,) = sqlx::query_as("SELECT pg_try_advisory_lock_shared($1)")
            .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
            .fetch_one(&mut conn)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!("Failed to probe maintenance advisory lock: {}", e))
            })?;
        if !shared_ok.0 {
            let _ = conn.close().await;
            return Ok(false);
        }
        let res = sqlx::query(
            "UPDATE _cluster_maintenance_barrier SET is_active = FALSE, lease_expires_at_nanos = 0 WHERE id = 1",
        )
        .execute(pool)
        .await;
        let _ = sqlx::query("SELECT pg_advisory_unlock_shared($1)")
            .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
            .execute(&mut conn)
            .await;
        let _ = conn.close().await;
        res.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to clear stale maintenance barrier row: {}",
                e
            ))
        })?;
        self.in_maintenance.store(false, Ordering::SeqCst);
        Ok(true)
    }

    async fn rollback_failed_entry(
        &self,
        conn_opt: Option<PgConnection>,
        prev_fencing_gen: u64,
        orig_err: ForgeError,
    ) -> ForgeError {
        self.in_maintenance.store(false, Ordering::SeqCst);
        let mut synced_gen = prev_fencing_gen;

        let mut rollback_errors: Vec<String> = Vec::new();
        if let Some(mut conn) = conn_opt {
            if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                .execute(&mut conn)
                .await
            {
                rollback_errors.push(format!("advisory unlock failed: {}", e));
            }
            let _ = conn.close().await;
        }
        if let Some(pool) = &self.pool {
            let clear_res = sqlx::query(
                "UPDATE _cluster_maintenance_barrier SET is_active = FALSE, lease_expires_at_nanos = 0 WHERE id = 1",
            )
            .execute(pool)
            .await;
            if let Err(e) = clear_res {
                rollback_errors.push(format!(
                    "failed to clear _cluster_maintenance_barrier.is_active: {}",
                    e
                ));
            }

            // Reconcile and verify authoritative barrier state & monotonic fencing generation
            let verify_res: Result<Option<(bool, i64)>, _> = sqlx::query_as(
                "SELECT is_active, fencing_generation FROM _cluster_maintenance_barrier WHERE id = 1",
            )
            .fetch_optional(pool)
            .await;
            match verify_res {
                Ok(Some((still_active, db_gen))) => {
                    synced_gen = synced_gen.max(db_gen as u64);
                    if still_active {
                        rollback_errors.push(
                            "verification confirmed _cluster_maintenance_barrier.is_active is still TRUE after rollback"
                                .to_string(),
                        );
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    rollback_errors.push(format!(
                        "failed to verify _cluster_maintenance_barrier state after rollback: {}",
                        e
                    ));
                }
            }
        }
        self.fencing_generation.store(synced_gen, Ordering::SeqCst);

        if rollback_errors.is_empty() {
            orig_err
        } else {
            ForgeError::Internal(format!(
                "{} AND maintenance entry rollback failed: [{}]",
                orig_err,
                rollback_errors.join("; ")
            ))
        }
    }

    /// Enter maintenance mode and drain in-flight operations across all cluster processes.
    pub async fn enter_maintenance(
        &self,
        drain_timeout: Duration,
    ) -> Result<MaintenanceGuard, ForgeError> {
        self.enter_maintenance_with_options(
            drain_timeout,
            DEFAULT_MAINTENANCE_LEASE_TTL,
            "SELECT pg_try_advisory_lock($1)",
        )
        .await
    }

    /// Enter maintenance mode with an explicit bounded lease TTL.
    pub async fn enter_maintenance_with_lease(
        &self,
        drain_timeout: Duration,
        lease_ttl: Duration,
    ) -> Result<MaintenanceGuard, ForgeError> {
        self.enter_maintenance_with_options(
            drain_timeout,
            lease_ttl,
            "SELECT pg_try_advisory_lock($1)",
        )
        .await
    }

    /// Enter maintenance mode using a caller-specified advisory lock query (used by fault-injection tests).
    pub async fn enter_maintenance_with_lock_sql(
        &self,
        drain_timeout: Duration,
        lock_sql: &str,
    ) -> Result<MaintenanceGuard, ForgeError> {
        self.enter_maintenance_with_options(drain_timeout, DEFAULT_MAINTENANCE_LEASE_TTL, lock_sql)
            .await
    }

    async fn enter_maintenance_with_options(
        &self,
        drain_timeout: Duration,
        lease_ttl: Duration,
        lock_sql: &str,
    ) -> Result<MaintenanceGuard, ForgeError> {
        let prev_fencing_gen = self.fencing_generation.load(Ordering::SeqCst);
        self.in_maintenance.store(true, Ordering::SeqCst);
        let in_memory_gen = self.fencing_generation.fetch_add(1, Ordering::SeqCst) + 1;
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let lease_expires_at_nanos =
            now_nanos.saturating_add(i64::try_from(lease_ttl.as_nanos()).unwrap_or(i64::MAX));

        let mut guard_conn = None;
        let mut final_fencing_gen = in_memory_gen;

        if let Some(opts) = &self.connect_options {
            if let Some(pool) = &self.pool {
                // 1. Mark barrier active, set bounded lease expiry, and increment monotonic fencing generation
                let barrier_res: Result<(i64,), _> = sqlx::query_as(
                    r#"
                    INSERT INTO _cluster_maintenance_barrier (id, is_active, activated_at_nanos, activated_by, reason, fencing_generation, lease_expires_at_nanos)
                    VALUES (1, TRUE, $1, 'spine-backup', 'Coordinated deployment-wide multi-volume backup', 2, $2)
                    ON CONFLICT (id) DO UPDATE SET 
                        is_active = TRUE,
                        activated_at_nanos = $1,
                        activated_by = 'spine-backup',
                        reason = 'Coordinated deployment-wide multi-volume backup',
                        fencing_generation = _cluster_maintenance_barrier.fencing_generation + 1,
                        lease_expires_at_nanos = $2
                    RETURNING fencing_generation
                    "#,
                )
                .bind(now_nanos)
                .bind(lease_expires_at_nanos)
                .fetch_one(pool)
                .await;

                match barrier_res {
                    Ok((gen,)) => {
                        final_fencing_gen = gen as u64;
                        self.fencing_generation
                            .store(final_fencing_gen, Ordering::SeqCst);
                    }
                    Err(e) => {
                        // Fallback: If table/column does not exist, ensure table & column exist, then retry
                        let create_res = sqlx::query(
                            r#"
                            CREATE TABLE IF NOT EXISTS _cluster_maintenance_barrier (
                                id INT PRIMARY KEY DEFAULT 1,
                                is_active BOOLEAN NOT NULL DEFAULT FALSE,
                                activated_at_nanos BIGINT NOT NULL DEFAULT 0,
                                activated_by VARCHAR(255) NOT NULL DEFAULT '',
                                reason VARCHAR(255) NOT NULL DEFAULT '',
                                fencing_generation BIGINT NOT NULL DEFAULT 1,
                                lease_expires_at_nanos BIGINT NOT NULL DEFAULT 0
                            )
                            "#,
                        )
                        .execute(pool)
                        .await;
                        let _ = sqlx::query(
                            "ALTER TABLE _cluster_maintenance_barrier ADD COLUMN IF NOT EXISTS lease_expires_at_nanos BIGINT NOT NULL DEFAULT 0",
                        )
                        .execute(pool)
                        .await;

                        if create_res.is_ok() {
                            let gen_res: Result<(i64,), _> = sqlx::query_as(
                                r#"
                                INSERT INTO _cluster_maintenance_barrier (id, is_active, activated_at_nanos, activated_by, reason, fencing_generation, lease_expires_at_nanos)
                                VALUES (1, TRUE, $1, 'spine-backup', 'Coordinated deployment-wide multi-volume backup', 2, $2)
                                ON CONFLICT (id) DO UPDATE SET 
                                    is_active = TRUE,
                                    activated_at_nanos = $1,
                                    activated_by = 'spine-backup',
                                    reason = 'Coordinated deployment-wide multi-volume backup',
                                    fencing_generation = _cluster_maintenance_barrier.fencing_generation + 1,
                                    lease_expires_at_nanos = $2
                                RETURNING fencing_generation
                                "#,
                            )
                            .bind(now_nanos)
                            .bind(lease_expires_at_nanos)
                            .fetch_one(pool)
                            .await;

                            match gen_res {
                                Ok((gen,)) => {
                                    final_fencing_gen = gen as u64;
                                    self.fencing_generation
                                        .store(final_fencing_gen, Ordering::SeqCst);
                                }
                                Err(ins_err) => {
                                    // Uncertain DB state: original or fallback statement may have committed server-side!
                                    let orig = ForgeError::Internal(format!(
                                        "Failed to update cluster maintenance barrier (original: {}; fallback: {})",
                                        e, ins_err
                                    ));
                                    return Err(self
                                        .rollback_failed_entry(None, prev_fencing_gen, orig)
                                        .await);
                                }
                            }
                        } else {
                            // Uncertain DB state: original barrier_res may have committed before response was lost!
                            let orig = ForgeError::Internal(format!(
                                "Failed to update cluster maintenance barrier: {}",
                                e
                            ));
                            return Err(self
                                .rollback_failed_entry(None, prev_fencing_gen, orig)
                                .await);
                        }
                    }
                }
            }

            // 2. Dedicated non-pooled connection to acquire exclusive advisory lock
            let mut conn = match PgConnection::connect_with(opts.as_ref()).await {
                Ok(c) => c,
                Err(e) => {
                    let orig = ForgeError::Internal(format!(
                        "Failed to establish dedicated connection for maintenance lock: {}",
                        e
                    ));
                    return Err(self
                        .rollback_failed_entry(None, prev_fencing_gen, orig)
                        .await);
                }
            };

            // 3. Acquire exclusive advisory lock, waiting for existing in-flight shared locks to drain
            let start = std::time::Instant::now();
            let mut acquired = false;
            while start.elapsed() < drain_timeout {
                let lock_res: (bool,) = match sqlx::query_as(lock_sql)
                    .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                    .fetch_one(&mut conn)
                    .await
                {
                    Ok(v) => v,
                    Err(e) => {
                        let orig = ForgeError::Internal(format!(
                            "Failed to query exclusive maintenance lock: {}",
                            e
                        ));
                        return Err(self
                            .rollback_failed_entry(Some(conn), prev_fencing_gen, orig)
                            .await);
                    }
                };

                if lock_res.0 {
                    acquired = true;
                    break;
                }
                sleep(Duration::from_millis(50)).await;
            }

            if !acquired {
                let orig = ForgeError::Internal(format!(
                    "Timed out after {:?} waiting for in-flight writers to drain across cluster",
                    drain_timeout
                ));
                return Err(self
                    .rollback_failed_entry(Some(conn), prev_fencing_gen, orig)
                    .await);
            }

            guard_conn = Some(conn);
        }

        tracing::info!(
            "Maintenance consistency barrier activated at generation {}. Draining in-flight writes for {:?}...",
            final_fencing_gen,
            drain_timeout
        );

        let start = std::time::Instant::now();
        while self.in_flight_writes.load(Ordering::SeqCst) > 0 && start.elapsed() < drain_timeout {
            sleep(Duration::from_millis(20)).await;
        }
        if self.in_flight_writes.load(Ordering::SeqCst) > 0 {
            let orig = ForgeError::Internal(format!(
                "Timed out after {:?} waiting for {} local in-flight writers to drain",
                drain_timeout,
                self.in_flight_writes.load(Ordering::SeqCst)
            ));
            return Err(self
                .rollback_failed_entry(guard_conn, prev_fencing_gen, orig)
                .await);
        }

        Ok(MaintenanceGuard {
            barrier: self.clone(),
            conn: guard_conn,
            fencing_generation: final_fencing_gen,
            released: false,
        })
    }
}

pub struct WritePermit {
    conn: Option<PgConnection>,
    counter: Arc<AtomicUsize>,
    pub permit_generation: u64,
    released: bool,
}

impl WritePermit {
    /// Explicitly release the write permit and shared advisory lock.
    pub async fn release(mut self) -> Result<(), ForgeError> {
        if !self.released {
            self.counter.fetch_sub(1, Ordering::SeqCst);
            self.released = true;
        }
        if let Some(mut conn) = self.conn.take() {
            let _ = sqlx::query("SELECT pg_advisory_unlock_shared($1)")
                .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                .execute(&mut conn)
                .await;
            let _ = conn.close().await;
        }
        Ok(())
    }

    /// Verify that the dedicated database connection and advisory lock remain active,
    /// and that the cluster has not entered maintenance or advanced the fencing generation.
    pub async fn verify_active(&mut self) -> Result<(), ForgeError> {
        if let Some(conn) = &mut self.conn {
            let barrier_row: Result<Option<(bool, i64)>, _> = sqlx::query_as(
                "SELECT is_active, fencing_generation FROM _cluster_maintenance_barrier WHERE id = 1",
            )
            .fetch_optional(conn)
            .await;

            match barrier_row {
                Ok(Some((is_active, active_gen))) => {
                    if is_active || active_gen as u64 != self.permit_generation {
                        return Err(ForgeError::ServiceUnavailable(format!(
                            "Write permit fencing violation: cluster is in maintenance or fencing generation advanced (permit generation {}, active generation {})",
                            self.permit_generation, active_gen
                        )));
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    return Err(ForgeError::ServiceUnavailable(format!(
                        "Writer permit connection or barrier verification failed: {}",
                        e
                    )));
                }
            }
        }
        Ok(())
    }
}

impl Drop for WritePermit {
    fn drop(&mut self) {
        if !self.released {
            self.counter.fetch_sub(1, Ordering::SeqCst);
            self.released = true;
        }
    }
}

pub struct MaintenanceGuard {
    barrier: ConsistencyBarrier,
    conn: Option<PgConnection>,
    pub fencing_generation: u64,
    released: bool,
}

impl MaintenanceGuard {
    pub fn fencing_generation(&self) -> u64 {
        self.fencing_generation
    }

    /// Explicitly release the maintenance guard, deactivating the barrier and unlocking the advisory lock.
    pub async fn release(mut self) -> Result<(), ForgeError> {
        self.released = true;
        self.barrier.in_maintenance.store(false, Ordering::SeqCst);
        let mut release_errors: Vec<String> = Vec::new();
        if let Some(pool) = &self.barrier.pool {
            if let Err(e) = sqlx::query(
                "UPDATE _cluster_maintenance_barrier SET is_active = FALSE, lease_expires_at_nanos = 0 WHERE id = 1",
            )
            .execute(pool)
            .await
            {
                release_errors.push(format!(
                    "Failed to clear _cluster_maintenance_barrier.is_active: {}",
                    e
                ));
            }
        }
        if let Some(mut conn) = self.conn.take() {
            if let Err(e) = sqlx::query("SELECT pg_advisory_unlock($1)")
                .bind(SPINE_CLUSTER_MAINTENANCE_LOCK_ID)
                .execute(&mut conn)
                .await
            {
                release_errors.push(format!(
                    "Failed to release PostgreSQL maintenance advisory lock: {}",
                    e
                ));
            }
            let _ = conn.close().await;
        }
        if !release_errors.is_empty() {
            return Err(ForgeError::Internal(format!(
                "MaintenanceGuard release failed: {}",
                release_errors.join("; ")
            )));
        }
        tracing::info!("Maintenance consistency barrier explicitly released.");
        Ok(())
    }
}

impl Drop for MaintenanceGuard {
    fn drop(&mut self) {
        if !self.released {
            self.barrier.in_maintenance.store(false, Ordering::SeqCst);
            if let Some(pool) = self.barrier.pool.clone() {
                // Best-effort asynchronous update for barrier state table if explicit release was skipped
                tokio::spawn(async move {
                    let _ = sqlx::query(
                        "UPDATE _cluster_maintenance_barrier SET is_active = FALSE WHERE id = 1",
                    )
                    .execute(&pool)
                    .await;
                });
            }
            // When self.conn is dropped synchronously, the underlying TCP stream is closed,
            // causing PostgreSQL to automatically and immediately release all session advisory locks.
            tracing::info!("Maintenance consistency barrier guard dropped.");
        }
    }
}
