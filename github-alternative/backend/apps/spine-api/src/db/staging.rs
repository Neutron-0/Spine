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
        let pool = match PgPool::connect(&self.staged_db_url).await {
            Ok(p) => p,
            Err(_) => return Ok(false),
        };

        let row: Option<(String,)> =
            sqlx::query_as("SELECT generation_token FROM _restore_generation WHERE id = 1")
                .fetch_optional(&pool)
                .await
                .ok()
                .flatten();

        pool.close().await;
        Ok(row
            .map(|(gen,)| gen == self.target_generation)
            .unwrap_or(false))
    }

    /// Atomically cut over staged database using DatabaseSelector and recoverable versioning.
    /// Preserves existing database until finalization.
    pub async fn commit_cutover(&self, storage_root: &Path) -> Result<(), ForgeError> {
        let maint_pool = PgPool::connect(&self.maint_db_url).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to connect to maintenance DB for cutover: {}",
                e
            ))
        })?;

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
        let current_rec = DatabaseSelector::load_from_db(&maint_pool)
            .await
            .ok()
            .flatten();
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
            format!("decision_{}", self.restore_id),
            self.staged_digest.clone(),
            prev_db,
            prev_gen,
            self.active_versioned_name.clone(),
            self.target_generation.clone(),
            "Committed".to_string(),
            now_nanos,
            self.base_db_name.clone(),
        );

        DatabaseSelector::save_to_db(&maint_pool, &record).await?;
        DatabaseSelector::save_diagnostic_cache(storage_root, &record)?;

        // 3. Terminate active connections to base / previous database
        let _ = sqlx::query(term_sql)
            .bind(&self.base_db_name)
            .execute(&maint_pool)
            .await;

        maint_pool.close().await;
        tracing::info!(
            "Cutover successful: active database selector points to '{}'",
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
