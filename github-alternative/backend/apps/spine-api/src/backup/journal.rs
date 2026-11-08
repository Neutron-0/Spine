use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Discrete phases of the cross-resource restore state machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RestorePhase {
    /// Restore initialized: journal written, staging directory established.
    Initialized,
    /// Storage staged: archive unpacked and SHA-256 hashes verified in staging storage.
    StagedStorage,
    /// Database snapshot staged and verified.
    StagedDb,
    /// Database snapshot verified and restored to target database before storage publication.
    DbPublished,
    /// Backward-compatible alias for DbPublished
    #[serde(alias = "DatabaseRestored")]
    DatabaseRestored,
    /// Staged storage atomically published to live repository storage root.
    StoragePublished,
    /// Entire restore sequence completed and committed successfully.
    Committed,
    /// Backward-compatible alias for Committed
    #[serde(alias = "Completed")]
    Completed,
    /// Restore cleanly rolled back.
    RolledBack(String),
    /// Restore failed with specific diagnostic error.
    Failed(String),
}

/// Durable cross-resource restoration journal persisted as `.spine_restore_journal.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreJournal {
    pub restore_id: String,
    pub phase: RestorePhase,
    pub backup_id: String,
    pub input_archive: PathBuf,
    pub target_storage_root: PathBuf,
    pub staging_storage_root: PathBuf,
    pub db_snapshot_staged: Option<PathBuf>,
    pub previous_storage_generation: Option<String>,
    pub target_generation: String,
    pub started_at_nanos: i64,
    pub updated_at_nanos: i64,
}

impl RestoreJournal {
    pub const JOURNAL_FILE_NAME: &'static str = ".spine_restore_journal.json";
    pub const GENERATION_FILE_NAME: &'static str = ".spine_storage_generation";
    pub const DB_GENERATION_TABLE: &'static str = "_restore_generation";

    /// Initialize a new restoration journal tracking the state machine.
    pub fn new(
        input_archive: &Path,
        target_storage_root: &Path,
        staging_storage_root: &Path,
        backup_id: &str,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let previous_storage_generation = Self::read_storage_generation(target_storage_root);
        let target_generation = format!("gen_{}_{}", backup_id, uuid::Uuid::new_v4().simple());
        Self {
            restore_id: format!("rst_{}", uuid::Uuid::new_v4().simple()),
            phase: RestorePhase::Initialized,
            backup_id: backup_id.to_string(),
            input_archive: input_archive.to_path_buf(),
            target_storage_root: target_storage_root.to_path_buf(),
            staging_storage_root: staging_storage_root.to_path_buf(),
            db_snapshot_staged: None,
            previous_storage_generation,
            target_generation,
            started_at_nanos: now,
            updated_at_nanos: now,
        }
    }

    /// Read generation identity from live storage root.
    pub fn read_storage_generation(storage_root: &Path) -> Option<String> {
        let p = storage_root.join(Self::GENERATION_FILE_NAME);
        if p.exists() {
            std::fs::read_to_string(&p)
                .ok()
                .map(|s| s.trim().to_string())
        } else {
            None
        }
    }

    /// Write generation identity to live storage root.
    pub fn write_storage_generation(
        storage_root: &Path,
        generation: &str,
    ) -> Result<(), ForgeError> {
        let p = storage_root.join(Self::GENERATION_FILE_NAME);
        std::fs::write(&p, format!("{}\n", generation))
            .map_err(|e| ForgeError::Internal(format!("Failed to write storage generation: {}", e)))
    }

    /// Record generation token inside an active database transaction before committing.
    pub async fn record_db_generation_in_tx(
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        generation: &str,
        backup_id: &str,
    ) -> Result<(), ForgeError> {
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let create_sql = r#"
            CREATE TABLE IF NOT EXISTS _restore_generation (
                id INT PRIMARY KEY DEFAULT 1,
                generation_token VARCHAR(128) NOT NULL,
                backup_id VARCHAR(128) NOT NULL,
                restored_at_nanos BIGINT NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'COMMITTED'
            )
        "#;
        sqlx::query(create_sql)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to create _restore_generation table in transaction: {}",
                    e
                ))
            })?;

        let insert_sql = r#"
            INSERT INTO _restore_generation (id, generation_token, backup_id, restored_at_nanos, status)
            VALUES (1, $1, $2, $3, 'COMMITTED')
            ON CONFLICT (id) DO UPDATE SET
                generation_token = EXCLUDED.generation_token,
                backup_id = EXCLUDED.backup_id,
                restored_at_nanos = EXCLUDED.restored_at_nanos,
                status = EXCLUDED.status
        "#;
        sqlx::query(insert_sql)
            .bind(generation)
            .bind(backup_id)
            .bind(now_nanos)
            .execute(&mut **tx)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to record database restore generation in transaction: {}",
                    e
                ))
            })?;
        Ok(())
    }

    /// Record generation token in target database.
    pub async fn record_db_generation(
        pool: &sqlx::PgPool,
        generation: &str,
        backup_id: &str,
    ) -> Result<(), ForgeError> {
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let create_sql = r#"
            CREATE TABLE IF NOT EXISTS _restore_generation (
                id INT PRIMARY KEY DEFAULT 1,
                generation_token VARCHAR(128) NOT NULL,
                backup_id VARCHAR(128) NOT NULL,
                restored_at_nanos BIGINT NOT NULL,
                status VARCHAR(32) NOT NULL DEFAULT 'COMMITTED'
            )
        "#;
        sqlx::query(create_sql).execute(pool).await.map_err(|e| {
            ForgeError::Internal(format!("Failed to create _restore_generation table: {}", e))
        })?;

        let insert_sql = r#"
            INSERT INTO _restore_generation (id, generation_token, backup_id, restored_at_nanos, status)
            VALUES (1, $1, $2, $3, 'COMMITTED')
            ON CONFLICT (id) DO UPDATE SET
                generation_token = EXCLUDED.generation_token,
                backup_id = EXCLUDED.backup_id,
                restored_at_nanos = EXCLUDED.restored_at_nanos,
                status = EXCLUDED.status
        "#;
        sqlx::query(insert_sql)
            .bind(generation)
            .bind(backup_id)
            .bind(now_nanos)
            .execute(pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to record database restore generation: {}",
                    e
                ))
            })?;
        Ok(())
    }

    /// Return the canonical path to the journal file in target storage root.
    pub fn journal_path(target_storage_root: &Path) -> PathBuf {
        target_storage_root.join(Self::JOURNAL_FILE_NAME)
    }

    /// Load an existing restore journal if present in target storage root.
    pub fn load(target_storage_root: &Path) -> Result<Option<Self>, ForgeError> {
        let path = Self::journal_path(target_storage_root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path)
            .map_err(|e| ForgeError::Internal(format!("Failed to read restore journal: {}", e)))?;
        let journal: Self = serde_json::from_str(&content)
            .map_err(|e| ForgeError::Internal(format!("Corrupt restore journal: {}", e)))?;
        Ok(Some(journal))
    }

    /// Persist the updated state machine phase to disk with atomic write-and-rename.
    pub fn persist(&mut self, phase: RestorePhase) -> Result<(), ForgeError> {
        self.phase = phase;
        self.updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let path = Self::journal_path(&self.target_storage_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let _ = std::fs::create_dir_all(parent);

        let tmp_path = parent.join(format!(".tmp_jnl_{}.json", uuid::Uuid::new_v4().simple()));
        let json_data = serde_json::to_string_pretty(self)
            .map_err(|e| ForgeError::Internal(format!("Failed to serialize journal: {}", e)))?;
        std::fs::write(&tmp_path, json_data).map_err(|e| {
            ForgeError::Internal(format!("Failed to write temporary journal: {}", e))
        })?;
        std::fs::rename(&tmp_path, &path).map_err(|e| {
            ForgeError::Internal(format!("Failed to atomically publish journal: {}", e))
        })?;
        Ok(())
    }

    /// Remove the journal file upon successful completion.
    pub fn remove(target_storage_root: &Path) -> Result<(), ForgeError> {
        let path = Self::journal_path(target_storage_root);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        Ok(())
    }

    /// Synchronous journal recovery.
    /// Cleans up uncommitted staging artifacts if interrupted before database cutover.
    /// FAILS CLOSED with error if interrupted during or after publication.
    pub fn handle_startup_recovery_sync(storage_root: &Path) -> Result<Option<String>, ForgeError> {
        if let Some(journal) = Self::load(storage_root)? {
            match journal.phase {
                RestorePhase::Committed | RestorePhase::Completed => {
                    let _ = Self::remove(storage_root);
                    Ok(None)
                }
                RestorePhase::Initialized
                | RestorePhase::StagedStorage
                | RestorePhase::StagedDb
                | RestorePhase::RolledBack(_) => {
                    tracing::warn!(
                        "Detected interrupted restore session '{}' in phase '{:?}'. Cleaning up staging directory {:?}...",
                        journal.restore_id,
                        journal.phase,
                        journal.staging_storage_root
                    );
                    if journal.staging_storage_root.exists() {
                        let _ = std::fs::remove_dir_all(&journal.staging_storage_root);
                    }
                    let _ = Self::remove(storage_root);
                    Ok(Some(format!(
                        "Interrupted restore '{}' before publication was cleanly rolled back.",
                        journal.restore_id
                    )))
                }
                RestorePhase::DbPublished
                | RestorePhase::DatabaseRestored
                | RestorePhase::StoragePublished
                | RestorePhase::Failed(_) => {
                    tracing::error!(
                        "FATAL: Incomplete restore session '{}' detected in phase '{:?}'. Aborting startup.",
                        journal.restore_id,
                        journal.phase
                    );
                    Err(ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Incomplete restore session '{}' detected in phase '{:?}'. Database and storage cutover was incomplete. Startup failed closed to prevent split-brain data corruption.",
                        journal.restore_id,
                        journal.phase
                    )))
                }
            }
        } else {
            Ok(None)
        }
    }

    /// Asynchronous crash detection, cleanup and cross-resource generation verification.
    /// Checks both the durable journal file AND database generation identity.
    /// FAILS CLOSED if incomplete restoration or generation mismatch is detected.
    pub async fn handle_startup_recovery(
        storage_root: &Path,
        pool: Option<&sqlx::PgPool>,
    ) -> Result<Option<String>, ForgeError> {
        let sync_msg = Self::handle_startup_recovery_sync(storage_root)?;

        if let Some(pool) = pool {
            // Check if database is empty or initialized
            let table_count: (i64,) = sqlx::query_as(
                "SELECT count(*)::bigint FROM information_schema.tables WHERE table_schema = 'public' AND table_name NOT LIKE 'pg_%'",
            )
            .fetch_one(pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "FATAL STARTUP BLOCKED: Failed to inspect database schema catalog during recovery check: {}",
                    e
                ))
            })?;

            if table_count.0 > 0 {
                let row: Result<Option<(String,)>, _> =
                    sqlx::query_as("SELECT generation_token FROM _restore_generation WHERE id = 1")
                        .fetch_optional(pool)
                        .await;

                match row {
                    Ok(Some((db_gen,))) => {
                        let storage_gen = Self::read_storage_generation(storage_root);
                        match storage_gen {
                            Some(s_gen) if s_gen == db_gen => {
                                tracing::info!(
                                    "Verified consistent generation token '{}' across database and storage root.",
                                    db_gen
                                );
                            }
                            Some(s_gen) => {
                                return Err(ForgeError::Internal(format!(
                                    "FATAL STARTUP BLOCKED: Generation mismatch! Database generation is '{}' but storage generation is '{}'. Startup aborted to prevent split-brain data corruption.",
                                    db_gen, s_gen
                                )));
                            }
                            None => {
                                return Err(ForgeError::Internal(format!(
                                    "FATAL STARTUP BLOCKED: Database has generation '{}' but storage generation file is missing! Startup aborted to prevent split-brain data corruption.",
                                    db_gen
                                )));
                            }
                        }
                    }
                    Ok(None) => {
                        return Err(ForgeError::Internal(
                            "FATAL STARTUP BLOCKED: Non-empty database missing generation record in _restore_generation table. Startup aborted to prevent unaligned state.".to_string(),
                        ));
                    }
                    Err(e) => {
                        return Err(ForgeError::Internal(format!(
                            "FATAL STARTUP BLOCKED: Failed to query _restore_generation table: {}. Missing generation tables and query errors cannot be ignored.",
                            e
                        )));
                    }
                }
            }
        }

        Ok(sync_msg)
    }
}
