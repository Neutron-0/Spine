use crate::backup::ParticipantAcknowledgement;
use crate::core::error::ForgeError;
use crate::db::selector::{ActiveDatabaseRecord, DatabaseSelector};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;
use url::Url;

/// Managed staging session for database restoration with recoverable cutover.
pub struct StagedDatabaseRestore {
    pub restore_id: String,
    pub base_db_url: String,
    pub staged_db_url: String,
    pub maint_db_url: String,
    pub base_db_name: String,
    pub staged_db_name: String,
    pub active_versioned_name: String,
    pub old_db_name: String,
    pub target_generation: String,
    pub backup_id: String,
    pub staged_digest: String,
    pub staged_byte_length: u64,
    pub tables_restored: usize,
    pub rows_restored: u64,
    pub duration_ms: u64,
    pub acknowledgement: ParticipantAcknowledgement,
}

impl StagedDatabaseRestore {
    /// Prepare and validate an isolated database staging session before any live cutover.
    /// Strictly fails closed if isolated database staging cannot be established.
    /// Never restores into live database or live schemas.
    pub async fn prepare_from_file(
        base_db_url: &str,
        restore_id: &str,
        target_generation: &str,
        backup_id: &str,
        snapshot_file: &Path,
    ) -> Result<Self, ForgeError> {
        let f_meta = std::fs::metadata(snapshot_file).map_err(|e| {
            ForgeError::Internal(format!("Failed to read snapshot file metadata: {}", e))
        })?;
        let staged_byte_length = f_meta.len();

        let mut hasher = Sha256::new();
        let mut f_hash = File::open(snapshot_file).map_err(|e| {
            ForgeError::Internal(format!("Failed to open snapshot for digest: {}", e))
        })?;
        let mut h_buf = [0u8; 64 * 1024];
        loop {
            let n = f_hash
                .read(&mut h_buf)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            if n == 0 {
                break;
            }
            hasher.update(&h_buf[..n]);
        }
        let staged_digest = hex::encode(hasher.finalize());

        let parsed_url = Url::parse(base_db_url)
            .map_err(|e| ForgeError::Internal(format!("Invalid PostgreSQL URL: {}", e)))?;
        let base_db_name = parsed_url.path().trim_start_matches('/').to_string();
        if base_db_name.is_empty() {
            return Err(ForgeError::Internal(
                "PostgreSQL URL missing database name".into(),
            ));
        }

        let safe_restore_id = restore_id.replace('-', "_");
        let safe_base_name = base_db_name.replace('-', "_");
        let safe_target_gen = target_generation.replace('-', "_");
        let staged_db_name = format!("{}_staged_{}", safe_base_name, safe_restore_id);
        let active_versioned_name = format!("{}_gen_{}", safe_base_name, safe_target_gen);
        let old_db_name = format!("{}_old_{}", safe_base_name, safe_restore_id);

        let mut maint_url = parsed_url.clone();
        maint_url.set_path("postgres");
        let maint_url_str = maint_url.to_string();

        let mut staged_db_url = parsed_url.clone();
        staged_db_url.set_path(&staged_db_name);
        let staged_db_url_str = staged_db_url.to_string();

        // 1. Connect to maintenance database to create isolated staging database
        let maint_pool = PgPool::connect(&maint_url_str).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to PostgreSQL maintenance database to create isolated staging: {}. Staging aborted.",
                e
            ))
        })?;

        // Terminate any stale connections to staged_db_name if it was left from an interrupted previous attempt
        let term_sql = r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname = $1 AND pid <> pg_backend_pid()
        "#;
        let _ = sqlx::query(term_sql)
            .bind(&staged_db_name)
            .execute(&maint_pool)
            .await;

        let drop_sql = format!(r#"DROP DATABASE IF EXISTS "{}""#, staged_db_name);
        let _ = sqlx::query(&drop_sql).execute(&maint_pool).await;

        let create_sql = format!(r#"CREATE DATABASE "{}""#, staged_db_name);
        sqlx::query(&create_sql)
            .execute(&maint_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "FATAL: Failed to create isolated staging database '{}': {}. Restore cannot proceed without isolated staging.",
                    staged_db_name, e
                ))
            })?;

        maint_pool.close().await;

        tracing::info!(
            "Established isolated database-level staging: '{}'",
            staged_db_name
        );

        // 2. Connect to staged database and restore snapshot
        let staged_pool = PgPool::connect(&staged_db_url_str).await.map_err(|e| {
            ForgeError::Internal(format!("Failed to connect to staged database: {}", e))
        })?;

        let snap_file = File::open(snapshot_file).map_err(|e| {
            ForgeError::Internal(format!("Failed to open snapshot file for restore: {}", e))
        })?;
        let reader = BufReader::new(snap_file);

        let report = super::dump::restore_database_with_generation(
            &staged_pool,
            reader,
            Some(target_generation),
            Some(backup_id),
        )
        .await?;

        // 3. Verify staged database catalog and generation token
        let table_count: (i64,) = sqlx::query_as(
            "SELECT count(*)::bigint FROM information_schema.tables WHERE table_schema = 'public' AND table_name NOT LIKE 'pg_%'",
        )
        .fetch_one(&staged_pool)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to verify staged database tables: {}", e))
        })?;

        if table_count.0 == 0 {
            staged_pool.close().await;
            return Err(ForgeError::Internal(format!(
                "Staged database '{}' is empty after restore. Validation failed.",
                staged_db_name
            )));
        }

        let gen_row: Option<(String,)> =
            sqlx::query_as("SELECT generation_token FROM _restore_generation WHERE id = 1")
                .fetch_optional(&staged_pool)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!(
                        "Failed to query _restore_generation in staged database: {}",
                        e
                    ))
                })?;

        match gen_row {
            Some((ref gen,)) if gen == target_generation => {
                tracing::info!(
                    "Verified generation token '{}' in staged database '{}'",
                    gen,
                    staged_db_name
                );
            }
            Some((ref gen,)) => {
                staged_pool.close().await;
                return Err(ForgeError::Internal(format!(
                    "Staged database generation token mismatch: expected '{}', found '{}'",
                    target_generation, gen
                )));
            }
            None => {
                staged_pool.close().await;
                return Err(ForgeError::Internal(
                    "Staged database missing _restore_generation token. Validation failed."
                        .to_string(),
                ));
            }
        }

        // 4. Persist durable staging record inside the staged database
        let durable_record_id =
            format!("postgresql:{}.public._spine_staging_record", staged_db_name);
        let create_record_table = r#"
            CREATE TABLE IF NOT EXISTS _spine_staging_record (
                id INT PRIMARY KEY DEFAULT 1,
                restore_id TEXT NOT NULL,
                backup_id TEXT NOT NULL,
                target_generation TEXT NOT NULL,
                staged_digest TEXT NOT NULL,
                staged_byte_length BIGINT NOT NULL,
                tables_restored BIGINT NOT NULL,
                rows_restored BIGINT NOT NULL,
                prepared_at_nanos BIGINT NOT NULL
            );
        "#;
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        sqlx::query(create_record_table)
            .execute(&staged_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to create _spine_staging_record table: {}",
                    e
                ))
            })?;

        let insert_record = r#"
            INSERT INTO _spine_staging_record (id, restore_id, backup_id, target_generation, staged_digest, staged_byte_length, tables_restored, rows_restored, prepared_at_nanos)
            VALUES (1, $1, $2, $3, $4, $5, $6, $7, $8)
            ON CONFLICT (id) DO UPDATE SET
                restore_id = EXCLUDED.restore_id,
                backup_id = EXCLUDED.backup_id,
                target_generation = EXCLUDED.target_generation,
                staged_digest = EXCLUDED.staged_digest,
                staged_byte_length = EXCLUDED.staged_byte_length,
                tables_restored = EXCLUDED.tables_restored,
                rows_restored = EXCLUDED.rows_restored,
                prepared_at_nanos = EXCLUDED.prepared_at_nanos;
        "#;
        sqlx::query(insert_record)
            .bind(restore_id)
            .bind(backup_id)
            .bind(target_generation)
            .bind(&staged_digest)
            .bind(staged_byte_length as i64)
            .bind(report.tables_restored as i64)
            .bind(report.rows_restored as i64)
            .bind(now_nanos)
            .execute(&staged_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to record durable staging state in DB: {}",
                    e
                ))
            })?;

        staged_pool.close().await;

        let ack = ParticipantAcknowledgement {
            restore_id: restore_id.to_string(),
            backup_id: backup_id.to_string(),
            participant_id: "postgres_data".to_string(),
            volume_name: "postgres_data".to_string(),
            canonical_resource_id: format!("postgresql_database:{}", staged_db_name),
            previous_generation: None,
            target_generation: target_generation.to_string(),
            staged_digest: staged_digest.clone(),
            archive_payload_sha256: staged_digest.clone(),
            staged_resource_digest: staged_digest.clone(),
            staged_byte_length,
            prepared_at_nanos: now_nanos,
            prepared: true,
            durable_staging_record_id: durable_record_id,
            current_state: "StagedAndValidated".to_string(),
        };

        Ok(Self {
            restore_id: restore_id.to_string(),
            base_db_url: base_db_url.to_string(),
            staged_db_url: staged_db_url_str,
            maint_db_url: maint_url_str,
            base_db_name,
            staged_db_name,
            active_versioned_name,
            old_db_name,
            target_generation: target_generation.to_string(),
            backup_id: backup_id.to_string(),
            staged_digest,
            staged_byte_length,
            tables_restored: report.tables_restored,
            rows_restored: report.rows_restored as u64,
            duration_ms: report.duration_ms,
            acknowledgement: ack,
        })
    }

    /// Re-query preparation status directly from the staged database.
    pub async fn query_preparation_status(&self) -> Result<bool, ForgeError> {
        Ok(self.revalidate_staged_database().await.is_ok())
    }

    /// Exhaustively revalidate the staged database immediately before commit decision.
    /// Checks catalog presence, connects to staged DB, verifies durable staging record,
    /// verifies generation token, and confirms restored table inventory.
    pub async fn revalidate_staged_database(&self) -> Result<(), ForgeError> {
        // 1. Verify staged database exists in pg_database catalog
        let maint_pool = PgPool::connect(&self.maint_db_url).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to maintenance DB for staged DB verification: {}",
                e
            ))
        })?;
        let db_exists: Option<(bool,)> =
            sqlx::query_as("SELECT EXISTS (SELECT FROM pg_database WHERE datname = $1)")
                .bind(&self.staged_db_name)
                .fetch_optional(&maint_pool)
                .await
                .map_err(|e| {
                    ForgeError::Internal(format!("Failed to query pg_database catalog: {}", e))
                })?;

        if !db_exists.map(|r| r.0).unwrap_or(false) {
            return Err(ForgeError::Internal(format!(
                "Staged database '{}' does not exist in PostgreSQL pg_database catalog",
                self.staged_db_name
            )));
        }
        maint_pool.close().await;

        // 2. Connect directly to the staged database
        let pool = PgPool::connect(&self.staged_db_url).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to staged database '{}': {}",
                self.staged_db_name, e
            ))
        })?;

        // 3. Query and verify durable staging record
        let staging_row: Option<(String, String, String, String, i64, i64, i64)> = sqlx::query_as(
            "SELECT restore_id, backup_id, target_generation, staged_digest, staged_byte_length, tables_restored, rows_restored \
             FROM _spine_staging_record WHERE id = 1",
        )
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to query _spine_staging_record in staged DB: {}",
                e
            ))
        })?;

        match staging_row {
            Some((r_id, b_id, t_gen, s_dig, _s_bytes, tables, rows)) => {
                if r_id != self.restore_id {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record restore_id mismatch: expected '{}', got '{}'",
                        self.restore_id, r_id
                    )));
                }
                if b_id != self.backup_id {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record backup_id mismatch: expected '{}', got '{}'",
                        self.backup_id, b_id
                    )));
                }
                if t_gen != self.target_generation {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record target_generation mismatch: expected '{}', got '{}'",
                        self.target_generation, t_gen
                    )));
                }
                if s_dig != self.staged_digest {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record digest mismatch: expected '{}', got '{}'",
                        self.staged_digest, s_dig
                    )));
                }
                if tables != self.tables_restored as i64 {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record tables_restored mismatch: expected {}, got {}",
                        self.tables_restored, tables
                    )));
                }
                if rows != self.rows_restored as i64 {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB record rows_restored mismatch: expected {}, got {}",
                        self.rows_restored, rows
                    )));
                }
            }
            None => {
                return Err(ForgeError::Internal(
                    "Staged database missing durable _spine_staging_record table/row".into(),
                ));
            }
        }

        // 4. Query and verify _restore_generation
        let gen_row: Option<(String, String, String)> = sqlx::query_as(
            "SELECT generation_token, backup_id, status FROM _restore_generation WHERE id = 1",
        )
        .fetch_optional(&pool)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to query _restore_generation in staged DB: {}",
                e
            ))
        })?;

        match gen_row {
            Some((gen, b_id, status)) => {
                if gen != self.target_generation {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB _restore_generation token mismatch: expected '{}', got '{}'",
                        self.target_generation, gen
                    )));
                }
                if b_id != self.backup_id {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB _restore_generation backup_id mismatch: expected '{}', got '{}'",
                        self.backup_id, b_id
                    )));
                }
                if status != "COMMITTED" {
                    return Err(ForgeError::Internal(format!(
                        "Staged DB _restore_generation status mismatch: expected 'COMMITTED', got '{}'",
                        status
                    )));
                }
            }
            None => {
                return Err(ForgeError::Internal(
                    "Staged database missing _restore_generation token".into(),
                ));
            }
        }

        // 5. Query table count in staged database
        let count_row: (i64,) = sqlx::query_as(
            "SELECT count(*)::bigint FROM information_schema.tables \
             WHERE table_schema = 'public' AND table_name NOT LIKE 'pg_%' AND table_name NOT LIKE '_spine_%'",
        )
        .fetch_one(&pool)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!("Failed to inspect staged DB table catalog: {}", e))
        })?;

        if count_row.0 != self.tables_restored as i64 {
            return Err(ForgeError::Internal(format!(
                "Staged DB catalog table count mismatch: expected {} restored tables, found {}",
                self.tables_restored, count_row.0
            )));
        }

        pool.close().await;
        Ok(())
    }

    /// Atomically cut over staged database using DatabaseSelector and recoverable versioning.
    /// Preserves existing database until finalization.
    pub async fn commit_cutover(
        &self,
        storage_root: &Path,
        decision: &crate::backup::CommitDecisionRecord,
    ) -> Result<(), ForgeError> {
        if decision.restore_id != self.restore_id {
            return Err(ForgeError::Internal(format!(
                "Cannot commit cutover: decision restore_id '{}' != staging restore_id '{}'",
                decision.restore_id, self.restore_id
            )));
        }
        if decision.backup_id != self.backup_id {
            return Err(ForgeError::Internal(format!(
                "Cannot commit cutover: decision backup_id '{}' != staging backup_id '{}'",
                decision.backup_id, self.backup_id
            )));
        }
        if decision.new_generation != self.target_generation {
            return Err(ForgeError::Internal(format!(
                "Cannot commit cutover: decision new_generation '{}' != staging target_generation '{}'",
                decision.new_generation, self.target_generation
            )));
        }
        if decision.decision_token.trim().is_empty() {
            return Err(ForgeError::Internal(
                "Cannot commit cutover: decision_token is empty".into(),
            ));
        }

        let maint_pool = PgPool::connect(&self.maint_db_url).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to maintenance DB for cutover: {}",
                e
            ))
        })?;

        // Check idempotency: if active record already matches this decision token and db name, cutover succeeded
        let current_rec = DatabaseSelector::load_from_db(&maint_pool)
            .await
            .ok()
            .flatten();
        if let Some(ref r) = current_rec {
            if r.decision_id == decision.decision_token
                && r.active_db_name == self.active_versioned_name
            {
                tracing::info!(
                    "Database cutover already committed for decision_token '{}'",
                    decision.decision_token
                );
                let _ = DatabaseSelector::save_diagnostic_cache(storage_root, r);
                maint_pool.close().await;
                return Ok(());
            }
        }

        // 1. Rename staged database to authoritative active_versioned_name
        let term_sql = r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname = $1 AND pid <> pg_backend_pid()
        "#;
        let _ = sqlx::query(term_sql)
            .bind(&self.active_versioned_name)
            .execute(&maint_pool)
            .await;
        let _ = sqlx::query(&format!(
            r#"DROP DATABASE IF EXISTS "{}""#,
            self.active_versioned_name
        ))
        .execute(&maint_pool)
        .await;

        let rename_staged = format!(
            r#"ALTER DATABASE "{}" RENAME TO "{}""#,
            self.staged_db_name, self.active_versioned_name
        );
        sqlx::query(&rename_staged)
            .execute(&maint_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to rename staged database to versioned database: {}",
                    e
                ))
            })?;

        // 2. Durably record the active database selection in the control table and storage root
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let prev_seq = current_rec.as_ref().map(|r| r.sequence_number).unwrap_or(0);
        let prev_db = current_rec
            .as_ref()
            .map(|r| r.active_db_name.clone())
            .unwrap_or_else(|| self.base_db_name.clone());
        let prev_gen = current_rec
            .as_ref()
            .map(|r| r.target_generation.clone())
            .unwrap_or_else(|| "0".to_string());

        let record = ActiveDatabaseRecord::new_record(
            prev_seq + 1,
            self.restore_id.clone(),
            decision.decision_token.clone(),
            self.staged_digest.clone(),
            prev_db,
            prev_gen,
            self.active_versioned_name.clone(),
            self.target_generation.clone(),
            "Committed".to_string(),
            now_nanos,
            self.base_db_name.clone(),
        );

        DatabaseSelector::save_to_db_transactional(&maint_pool, &record, Some(prev_seq)).await?;
        let _ = DatabaseSelector::save_diagnostic_cache(storage_root, &record);

        // 3. Terminate active connections to base / previous database
        let _ = sqlx::query(term_sql)
            .bind(&self.base_db_name)
            .execute(&maint_pool)
            .await;

        maint_pool.close().await;
        tracing::info!(
            "Cutover successful: active database selector bound to decision '{}' points to '{}'",
            decision.decision_token,
            self.active_versioned_name
        );
        Ok(())
    }

    /// Roll back database cutover, dropping staged resources if needed.
    pub async fn rollback(&self) -> Result<(), ForgeError> {
        if let Ok(maint_pool) = PgPool::connect(&self.maint_db_url).await {
            let term_sql = r#"
                SELECT pg_terminate_backend(pid)
                FROM pg_stat_activity
                WHERE datname = $1 AND pid <> pg_backend_pid()
            "#;
            let _ = sqlx::query(term_sql)
                .bind(&self.staged_db_name)
                .execute(&maint_pool)
                .await;
            let _ = sqlx::query(&format!(
                r#"DROP DATABASE IF EXISTS "{}""#,
                self.staged_db_name
            ))
            .execute(&maint_pool)
            .await;

            let _ = sqlx::query(term_sql)
                .bind(&self.active_versioned_name)
                .execute(&maint_pool)
                .await;
            let _ = sqlx::query(&format!(
                r#"DROP DATABASE IF EXISTS "{}""#,
                self.active_versioned_name
            ))
            .execute(&maint_pool)
            .await;

            maint_pool.close().await;
        }
        Ok(())
    }

    /// Finalize database cutover: cleanly drop old database generations after global verification.
    pub async fn finalize(&self) -> Result<(), ForgeError> {
        let maint_pool = PgPool::connect(&self.maint_db_url).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to maintenance DB for finalization: {}",
                e
            ))
        })?;

        let term_sql = r#"
            SELECT pg_terminate_backend(pid)
            FROM pg_stat_activity
            WHERE datname = $1 AND pid <> pg_backend_pid()
        "#;
        let _ = sqlx::query(term_sql)
            .bind(&self.old_db_name)
            .execute(&maint_pool)
            .await;

        let drop_sql = format!(r#"DROP DATABASE IF EXISTS "{}""#, self.old_db_name);
        let _ = sqlx::query(&drop_sql).execute(&maint_pool).await;

        maint_pool.close().await;
        Ok(())
    }
}
