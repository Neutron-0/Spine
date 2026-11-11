use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use url::Url;

pub const SELECTOR_FILE_NAME: &str = "active_database_selector.json";
pub const CONTROL_DIR_NAME: &str = ".spine_restore_control";
pub const SELECTOR_TABLE_NAME: &str = "_spine_active_database_selector";

/// Authoritative record identifying the active PostgreSQL database.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveDatabaseRecord {
    pub selector_schema_version: u32,
    pub sequence_number: u64,
    pub restore_id: String,
    pub decision_id: String,
    pub backup_digest: String,
    pub previous_db_name: String,
    pub previous_generation: String,
    pub active_db_name: String,
    pub target_generation: String,
    #[serde(default)]
    pub active_generation: String,
    pub publication_state: String,
    pub integrity_hash: String,
    pub cutover_at_nanos: i64,
    pub updated_at_nanos: i64,
    pub base_db_name: String,
}

impl ActiveDatabaseRecord {
    pub fn active_generation(&self) -> &str {
        &self.active_generation
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compute_integrity_hash(
        selector_schema_version: u32,
        sequence_number: u64,
        restore_id: &str,
        decision_id: &str,
        backup_digest: &str,
        previous_db_name: &str,
        previous_generation: &str,
        active_db_name: &str,
        target_generation: &str,
        publication_state: &str,
        cutover_at_nanos: i64,
        updated_at_nanos: i64,
        base_db_name: &str,
    ) -> String {
        use sha2::{Digest, Sha256};
        let canonical_str = format!(
            "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{}",
            selector_schema_version,
            sequence_number,
            restore_id,
            decision_id,
            backup_digest,
            previous_db_name,
            previous_generation,
            active_db_name,
            target_generation,
            publication_state,
            cutover_at_nanos,
            updated_at_nanos,
            base_db_name
        );
        let mut hasher = Sha256::new();
        hasher.update(canonical_str.as_bytes());
        hex::encode(hasher.finalize())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_record(
        sequence_number: u64,
        restore_id: String,
        decision_id: String,
        backup_digest: String,
        previous_db_name: String,
        previous_generation: String,
        active_db_name: String,
        target_generation: String,
        publication_state: String,
        cutover_at_nanos: i64,
        base_db_name: String,
    ) -> Self {
        let updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let schema_ver = 1;
        let integrity_hash = Self::compute_integrity_hash(
            schema_ver,
            sequence_number,
            &restore_id,
            &decision_id,
            &backup_digest,
            &previous_db_name,
            &previous_generation,
            &active_db_name,
            &target_generation,
            &publication_state,
            cutover_at_nanos,
            updated_at_nanos,
            &base_db_name,
        );
        Self {
            selector_schema_version: schema_ver,
            sequence_number,
            restore_id,
            decision_id,
            backup_digest,
            previous_db_name,
            previous_generation,
            active_db_name,
            active_generation: target_generation.clone(),
            target_generation,
            publication_state,
            integrity_hash,
            cutover_at_nanos,
            updated_at_nanos,
            base_db_name,
        }
    }

    pub fn verify_integrity(&self) -> Result<(), ForgeError> {
        if self.selector_schema_version != 1 {
            return Err(ForgeError::Internal(format!(
                "Unsupported selector schema version: {}",
                self.selector_schema_version
            )));
        }
        let expected = Self::compute_integrity_hash(
            self.selector_schema_version,
            self.sequence_number,
            &self.restore_id,
            &self.decision_id,
            &self.backup_digest,
            &self.previous_db_name,
            &self.previous_generation,
            &self.active_db_name,
            &self.target_generation,
            &self.publication_state,
            self.cutover_at_nanos,
            self.updated_at_nanos,
            &self.base_db_name,
        );
        if self.integrity_hash != expected {
            return Err(ForgeError::Internal(format!(
                "Authoritative selector record integrity hash mismatch: expected '{}', got '{}'",
                expected, self.integrity_hash
            )));
        }
        Ok(())
    }
}

pub struct DatabaseSelector;

impl DatabaseSelector {
    /// Return the control file path in the storage root.
    pub fn selector_file_path(storage_root: &Path) -> PathBuf {
        storage_root.join(CONTROL_DIR_NAME).join(SELECTOR_FILE_NAME)
    }

    /// Read the active database record from disk diagnostic cache.
    pub fn load_diagnostic_cache(
        storage_root: &Path,
    ) -> Result<Option<ActiveDatabaseRecord>, ForgeError> {
        let path = Self::selector_file_path(storage_root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path).map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to read active database selector cache: {}",
                e
            ))
        })?;
        let record = serde_json::from_str::<ActiveDatabaseRecord>(&content).map_err(|e| {
            ForgeError::Internal(format!("Corrupt active database selector cache: {}", e))
        })?;
        record.verify_integrity()?;
        Ok(Some(record))
    }

    /// Backwards compatibility alias for load_diagnostic_cache.
    pub fn load_from_file(storage_root: &Path) -> Result<Option<ActiveDatabaseRecord>, ForgeError> {
        Self::load_diagnostic_cache(storage_root)
    }

    /// Persist the active database record to disk diagnostic cache using atomic replacement and sync.
    pub fn save_diagnostic_cache(
        storage_root: &Path,
        record: &ActiveDatabaseRecord,
    ) -> Result<(), ForgeError> {
        record.verify_integrity()?;
        let path = Self::selector_file_path(storage_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;

        let json = serde_json::to_string_pretty(record).map_err(|e| {
            ForgeError::Internal(format!("Failed to serialize database selector: {}", e))
        })?;
        let tmp = parent.join(format!(".tmp_sel_{}.json", uuid::Uuid::new_v4().simple()));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| {
                ForgeError::Internal(format!("Failed to create temporary selector cache: {}", e))
            })?;
            f.write_all(json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            f.flush().map_err(|e| ForgeError::Internal(e.to_string()))?;
            f.sync_all()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        crate::backup::safe_atomic_replace(&tmp, &path)?;
        Ok(())
    }

    /// Backwards compatibility alias for save_diagnostic_cache.
    pub fn save_to_file(
        storage_root: &Path,
        record: &ActiveDatabaseRecord,
    ) -> Result<(), ForgeError> {
        Self::save_diagnostic_cache(storage_root, record)
    }

    /// Query the dedicated control database table in PostgreSQL if available.
    #[allow(clippy::type_complexity)]
    pub async fn load_from_db(
        maint_pool: &PgPool,
    ) -> Result<Option<ActiveDatabaseRecord>, ForgeError> {
        let table_exists: Option<(bool,)> = sqlx::query_as(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(SELECTOR_TABLE_NAME)
        .fetch_optional(maint_pool)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to check selector table existence: {}", e))
        })?;

        if table_exists.map(|t| t.0).unwrap_or(false) {
            let row: Option<(
                i32,
                i64,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                String,
                i64,
                i64,
                String,
            )> = sqlx::query_as(
                "SELECT selector_schema_version, sequence_number, restore_id, decision_id, \
                        backup_digest, previous_db_name, previous_generation, active_db_name, \
                        target_generation, publication_state, integrity_hash, cutover_at_nanos, \
                        updated_at_nanos, base_db_name \
                 FROM _spine_active_database_selector WHERE id = 1",
            )
            .fetch_optional(maint_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to query _spine_active_database_selector: {}",
                    e
                ))
            })?;

            if let Some((
                ver,
                seq,
                rst,
                dec,
                bak_dig,
                prev_db,
                prev_gen,
                act_db,
                tgt_gen,
                pub_state,
                int_hash,
                cut_nanos,
                upd_nanos,
                base_db,
            )) = row
            {
                let rec = ActiveDatabaseRecord {
                    selector_schema_version: ver as u32,
                    sequence_number: seq as u64,
                    restore_id: rst,
                    decision_id: dec,
                    backup_digest: bak_dig,
                    previous_db_name: prev_db,
                    previous_generation: prev_gen,
                    active_db_name: act_db,
                    active_generation: tgt_gen.clone(),
                    target_generation: tgt_gen,
                    publication_state: pub_state,
                    integrity_hash: int_hash,
                    cutover_at_nanos: cut_nanos,
                    updated_at_nanos: upd_nanos,
                    base_db_name: base_db,
                };
                rec.verify_integrity()?;
                return Ok(Some(rec));
            }
        }
        Ok(None)
    }

    /// Persist the active database record to control database table in a single atomic transaction.
    pub async fn save_to_db(
        maint_pool: &PgPool,
        record: &ActiveDatabaseRecord,
    ) -> Result<(), ForgeError> {
        Self::save_to_db_transactional(maint_pool, record, None).await
    }

    /// Transactional monotonic compare-and-update for the active database selector.
    /// Strictly rejects sequence rollbacks, validates decision binding, and ensures idempotency.
    pub async fn save_to_db_transactional(
        maint_pool: &PgPool,
        record: &ActiveDatabaseRecord,
        expected_previous_seq: Option<u64>,
    ) -> Result<(), ForgeError> {
        record.verify_integrity()?;

        let ddl = format!(
            r#"
            CREATE TABLE IF NOT EXISTS "{}" (
                id INT PRIMARY KEY DEFAULT 1,
                selector_schema_version INT NOT NULL,
                sequence_number BIGINT NOT NULL,
                restore_id TEXT NOT NULL,
                decision_id TEXT NOT NULL,
                backup_digest TEXT NOT NULL,
                previous_db_name TEXT NOT NULL,
                previous_generation TEXT NOT NULL,
                active_db_name TEXT NOT NULL,
                target_generation TEXT NOT NULL,
                publication_state TEXT NOT NULL,
                integrity_hash TEXT NOT NULL,
                cutover_at_nanos BIGINT NOT NULL,
                updated_at_nanos BIGINT NOT NULL,
                base_db_name TEXT NOT NULL
            );
            "#,
            SELECTOR_TABLE_NAME
        );
        sqlx::query(&ddl)
            .execute(maint_pool)
            .await
            .map_err(|e| ForgeError::Internal(format!("Failed to create selector table: {}", e)))?;

        let mut tx = maint_pool.begin().await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to begin transaction for selector update: {}",
                e
            ))
        })?;

        // 1. Lock and inspect current selector row
        let lock_query = format!(
            r#"
            SELECT sequence_number, decision_id, active_db_name, target_generation
            FROM "{}"
            WHERE id = 1
            FOR UPDATE
            "#,
            SELECTOR_TABLE_NAME
        );
        let current_row: Option<(i64, String, String, String)> = sqlx::query_as(&lock_query)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!("Failed to lock selector row for update: {}", e))
            })?;

        if let Some((curr_seq, curr_dec, curr_db, curr_gen)) = current_row {
            let curr_seq_u64 = curr_seq as u64;

            // Idempotent retry of exact same decision
            if record.decision_id == curr_dec && record.sequence_number == curr_seq_u64 {
                if record.active_db_name != curr_db || record.target_generation != curr_gen {
                    return Err(ForgeError::Internal(format!(
                        "Contradictory decision replay: decision '{}' previously committed active_db '{}' gen '{}', but received active_db '{}' gen '{}'",
                        curr_dec, curr_db, curr_gen, record.active_db_name, record.target_generation
                    )));
                }
                tx.commit()
                    .await
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
                return Ok(());
            }

            // Reject sequence rollback or stale decision
            if record.sequence_number <= curr_seq_u64 {
                return Err(ForgeError::Internal(format!(
                    "FATAL SELECTOR ROLLBACK REJECTED: Candidate selector sequence {} is <= current authoritative sequence {}. Database sequence rollback is strictly forbidden.",
                    record.sequence_number, curr_seq_u64
                )));
            }

            // If expected previous sequence was specified, enforce exact CAS match
            if let Some(exp_prev) = expected_previous_seq {
                if curr_seq_u64 != exp_prev {
                    return Err(ForgeError::Internal(format!(
                        "Selector CAS sequence mismatch: expected previous sequence {}, but current authoritative sequence is {}",
                        exp_prev, curr_seq_u64
                    )));
                }
            }

            // Strictly enforce monotonic step of +1
            if record.sequence_number != curr_seq_u64 + 1 {
                return Err(ForgeError::Internal(format!(
                    "Selector sequence advancement must be strictly monotonic (+1): expected {}, got {}",
                    curr_seq_u64 + 1, record.sequence_number
                )));
            }
        } else {
            // First bootstrap insertion
            if record.sequence_number != 1 {
                return Err(ForgeError::Internal(format!(
                    "Initial selector record sequence number must be 1, got {}",
                    record.sequence_number
                )));
            }
        }

        // 2. Perform compare-and-update or insert
        let update_sql = format!(
            r#"
            INSERT INTO "{}" (
                id, selector_schema_version, sequence_number, restore_id, decision_id,
                backup_digest, previous_db_name, previous_generation, active_db_name,
                target_generation, publication_state, integrity_hash, cutover_at_nanos,
                updated_at_nanos, base_db_name
            )
            VALUES (1, $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)
            ON CONFLICT (id) DO UPDATE SET
                selector_schema_version = EXCLUDED.selector_schema_version,
                sequence_number = EXCLUDED.sequence_number,
                restore_id = EXCLUDED.restore_id,
                decision_id = EXCLUDED.decision_id,
                backup_digest = EXCLUDED.backup_digest,
                previous_db_name = EXCLUDED.previous_db_name,
                previous_generation = EXCLUDED.previous_generation,
                active_db_name = EXCLUDED.active_db_name,
                target_generation = EXCLUDED.target_generation,
                publication_state = EXCLUDED.publication_state,
                integrity_hash = EXCLUDED.integrity_hash,
                cutover_at_nanos = EXCLUDED.cutover_at_nanos,
                updated_at_nanos = EXCLUDED.updated_at_nanos,
                base_db_name = EXCLUDED.base_db_name
            WHERE "{}"."sequence_number" < EXCLUDED.sequence_number;
            "#,
            SELECTOR_TABLE_NAME, SELECTOR_TABLE_NAME
        );

        let res = sqlx::query(&update_sql)
            .bind(record.selector_schema_version as i32)
            .bind(record.sequence_number as i64)
            .bind(&record.restore_id)
            .bind(&record.decision_id)
            .bind(&record.backup_digest)
            .bind(&record.previous_db_name)
            .bind(&record.previous_generation)
            .bind(&record.active_db_name)
            .bind(&record.target_generation)
            .bind(&record.publication_state)
            .bind(&record.integrity_hash)
            .bind(record.cutover_at_nanos)
            .bind(record.updated_at_nanos)
            .bind(&record.base_db_name)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to record active database in control table: {}",
                    e
                ))
            })?;

        if res.rows_affected() == 0 {
            return Err(ForgeError::Internal(
                "Selector update affected 0 rows: concurrent update or sequence rollback prevented persistence".into(),
            ));
        }

        tx.commit().await.map_err(|e| {
            ForgeError::Internal(format!("Failed to commit selector transaction: {}", e))
        })?;

        // 3. Post-commit verification: verify database exists in pg_database catalog
        let db_exists: Option<(bool,)> =
            sqlx::query_as("SELECT EXISTS (SELECT FROM pg_database WHERE datname = $1)")
                .bind(&record.active_db_name)
                .fetch_optional(maint_pool)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to verify database existence in pg_database catalog: {}",
                        e
                    ))
                })?;

        if !db_exists.map(|r| r.0).unwrap_or(false) {
            return Err(ForgeError::Internal(format!(
                "Post-commit selector check failed: authoritative database '{}' does not exist in PostgreSQL pg_database catalog",
                record.active_db_name
            )));
        }

        Ok(())
    }

    /// Resolve the authoritative active database connection URL.
    /// Exactly ONE authority: PostgreSQL maintenance database table `_spine_active_database_selector`.
    /// Filesystem selector file is purely a diagnostic cache.
    pub async fn resolve_active_db_url(
        configured_url: &str,
        storage_root: Option<&Path>,
    ) -> Result<String, ForgeError> {
        let mut parsed = Url::parse(configured_url)
            .map_err(|e| ForgeError::Internal(format!("Invalid database URL: {}", e)))?;

        // Non-PostgreSQL URLs (e.g. SQLite in unit tests) are passed through directly
        if parsed.scheme() != "postgres" && parsed.scheme() != "postgresql" {
            return Ok(configured_url.to_string());
        }

        let base_db_name = parsed.path().trim_start_matches('/').to_string();
        if base_db_name.is_empty() {
            return Err(ForgeError::Internal(
                "PostgreSQL URL missing database name".into(),
            ));
        }

        // Connect to maintenance database
        let mut maint_url = parsed.clone();
        maint_url.set_path("postgres");
        let maint_pool = PgPool::connect(maint_url.as_str()).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to PostgreSQL maintenance database ('postgres') to resolve active database selector: {}",
                e
            ))
        })?;

        // 1. Query dedicated control database table
        let record_opt = Self::load_from_db(&maint_pool).await?;

        if let Some(record) = record_opt {
            if record.base_db_name != base_db_name {
                maint_pool.close().await;
                return Err(ForgeError::Internal(format!(
                    "Active database selector base_db_name mismatch: expected '{}', found '{}'",
                    base_db_name, record.base_db_name
                )));
            }

            if record.publication_state != "Committed" && record.publication_state != "Published" {
                maint_pool.close().await;
                return Err(ForgeError::Internal(format!(
                    "Active database publication state '{}' is not ready (must be Committed or Published)",
                    record.publication_state
                )));
            }

            // Verify active database actually exists in PostgreSQL
            let db_exists: (bool,) =
                sqlx::query_as("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                    .bind(&record.active_db_name)
                    .fetch_one(&maint_pool)
                    .await
                    .map_err(|e| {
                        ForgeError::Internal(format!(
                            "Failed to verify active database existence in pg_database: {}",
                            e
                        ))
                    })?;

            if !db_exists.0 {
                maint_pool.close().await;
                return Err(ForgeError::Internal(format!(
                    "Active database '{}' selected by authoritative selector does not exist in PostgreSQL. Failing closed.",
                    record.active_db_name
                )));
            }

            // Synchronize diagnostic cache on disk if storage root is accessible
            if let Some(root) = storage_root {
                let _ = Self::save_diagnostic_cache(root, &record);
            }

            parsed.set_path(&record.active_db_name);
            maint_pool.close().await;
            return Ok(parsed.to_string());
        }

        // 2. Table missing or no record present.
        // Check if base_db_name exists and has existing user tables.
        let base_exists: (bool,) =
            sqlx::query_as("SELECT EXISTS (SELECT 1 FROM pg_database WHERE datname = $1)")
                .bind(&base_db_name)
                .fetch_one(&maint_pool)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to check base database existence in pg_database: {}",
                        e
                    ))
                })?;

        if base_exists.0 {
            // Check if it contains existing user tables
            if let Ok(base_pool) = PgPool::connect(configured_url).await {
                let table_count: (i64,) = sqlx::query_as(
                    "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = 'public'",
                )
                .fetch_one(&base_pool)
                .await
                .unwrap_or((0,));
                base_pool.close().await;

                if table_count.0 > 0 {
                    maint_pool.close().await;
                    return Err(ForgeError::Internal(format!(
                        "FATAL: Existing database '{}' contains {} tables but lacks an authoritative selector record in the maintenance database. Refusing to operate unmanaged database without an authoritative selector.",
                        base_db_name, table_count.0
                    )));
                }
            }
        }

        // 3. Fresh bootstrap: 0 tables or base DB does not exist yet.
        // Initialize the selector table and bootstrap record.
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let bootstrap_record = ActiveDatabaseRecord::new_record(
            1,
            "bootstrap".to_string(),
            "bootstrap".to_string(),
            "bootstrap".to_string(),
            base_db_name.clone(),
            "0".to_string(),
            base_db_name.clone(),
            "1".to_string(),
            "Committed".to_string(),
            now_nanos,
            base_db_name.clone(),
        );

        Self::save_to_db(&maint_pool, &bootstrap_record).await?;
        if let Some(root) = storage_root {
            let _ = Self::save_diagnostic_cache(root, &bootstrap_record);
        }
        maint_pool.close().await;

        tracing::info!(
            "Bootstrapped authoritative active database selector pointing to '{}'",
            base_db_name
        );
        Ok(configured_url.to_string())
    }
}
