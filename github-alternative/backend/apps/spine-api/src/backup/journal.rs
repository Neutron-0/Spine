use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Durable preparation acknowledgement for an authoritative restore participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParticipantAcknowledgement {
    pub restore_id: String,
    pub backup_id: String,
    pub participant_id: String,
    pub volume_name: String,
    pub previous_generation: Option<String>,
    pub target_generation: String,
    pub staged_digest: String,
    pub staged_byte_length: u64,
    pub prepared_at_nanos: i64,
    pub prepared: bool,
}

/// Durable commit decision record persisted before any irreversible participant mutations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitDecisionRecord {
    pub restore_id: String,
    pub backup_id: String,
    pub old_generation: Option<String>,
    pub new_generation: String,
    pub required_participants: Vec<String>,
    pub staged_manifest_digests: BTreeMap<String, String>,
    pub decision_timestamp_nanos: i64,
    pub schema_identity: String,
    #[serde(default)]
    pub decision_token: String,
}

/// Durable publication record tracking cutover completion for an individual participant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParticipantPublicationRecord {
    pub participant_id: String,
    pub target_generation: String,
    pub published_at_nanos: i64,
    pub published: bool,
}

/// Discrete phases of the cross-resource restore state machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum RestorePhase {
    /// Restore initialized: journal written, staging directory established.
    Initialized,
    /// Storage staged: archive unpacked and SHA-256 hashes verified in staging storage.
    StagedStorage,
    /// Database snapshot staged and verified.
    StagedDb,
    /// All participant snapshots staged and verified in isolated temporary areas.
    Prepared,
    /// Durable global commit decision logged to disk before irreversible cutover begins.
    CommitDecision(CommitDecisionRecord),
    /// Participants currently publishing (tracks list of published participant IDs).
    Publishing { published_participants: Vec<String> },
    /// Database snapshot verified and restored to target database before storage publication.
    DbPublished,
    /// Backward-compatible alias for DbPublished
    #[serde(alias = "DatabaseRestored")]
    DatabaseRestored,
    /// Staged storage atomically published to live repository storage root.
    StoragePublished,
    /// All required participants published.
    Published,
    /// Post-cutover verification in progress or verified across all participants.
    Verified,
    /// Entire restore sequence completed, old generations purged, and committed successfully.
    Committed,
    /// Backward-compatible alias for Committed
    #[serde(alias = "Completed")]
    Completed,
    /// Restore cleanly aborted / rolled back before global commit decision.
    RolledBack(String),
    #[serde(alias = "Aborted")]
    Aborted(String),
    /// Restore failed with specific diagnostic error.
    Failed(String),
}

/// Durable cross-resource restoration journal persisted as `.spine_restore_control/restore_journal.json`.
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
    #[serde(default)]
    pub published_participants: Vec<String>,
    #[serde(default)]
    pub acknowledgements: BTreeMap<String, ParticipantAcknowledgement>,
    #[serde(default)]
    pub commit_decision: Option<CommitDecisionRecord>,
}

impl RestoreJournal {
    pub const CONTROL_DIR_NAME: &'static str = ".spine_restore_control";
    pub const JOURNAL_FILE_NAME: &'static str = "restore_journal.json";
    pub const LEGACY_JOURNAL_FILE_NAME: &'static str = ".spine_restore_journal.json";
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
            published_participants: Vec::new(),
            acknowledgements: BTreeMap::new(),
            commit_decision: None,
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

    /// Return the canonical path to the journal file in dedicated control directory.
    pub fn journal_path(target_storage_root: &Path) -> PathBuf {
        target_storage_root
            .join(Self::CONTROL_DIR_NAME)
            .join(Self::JOURNAL_FILE_NAME)
    }

    /// Return fallback last-valid journal path.
    pub fn last_valid_journal_path(target_storage_root: &Path) -> PathBuf {
        target_storage_root
            .join(Self::CONTROL_DIR_NAME)
            .join(format!("{}.last_valid", Self::JOURNAL_FILE_NAME))
    }

    /// Return legacy journal file path for backwards compatibility.
    pub fn legacy_journal_path(target_storage_root: &Path) -> PathBuf {
        target_storage_root.join(Self::LEGACY_JOURNAL_FILE_NAME)
    }

    /// Record participant preparation acknowledgement.
    pub fn record_participant_ack(
        &mut self,
        ack: ParticipantAcknowledgement,
    ) -> Result<(), ForgeError> {
        self.acknowledgements
            .insert(ack.participant_id.clone(), ack);
        self.persist(self.phase.clone())
    }

    /// Record participant publication progress.
    pub fn record_participant_published(&mut self, participant_id: &str) -> Result<(), ForgeError> {
        if !self
            .published_participants
            .iter()
            .any(|p| p == participant_id)
        {
            self.published_participants.push(participant_id.to_string());
        }
        self.persist(RestorePhase::Publishing {
            published_participants: self.published_participants.clone(),
        })
    }

    /// Load an existing restore journal if present in target storage control directory or root.
    /// Resiliently falls back to last_valid journal backup if primary journal is corrupt or unreadable.
    pub fn load(target_storage_root: &Path) -> Result<Option<Self>, ForgeError> {
        let path = Self::journal_path(target_storage_root);
        let last_valid = Self::last_valid_journal_path(target_storage_root);
        let legacy = Self::legacy_journal_path(target_storage_root);

        let active_path = if path.exists() {
            path
        } else if last_valid.exists() {
            last_valid.clone()
        } else if legacy.exists() {
            legacy
        } else {
            return Ok(None);
        };

        let content = match std::fs::read_to_string(&active_path) {
            Ok(c) => c,
            Err(_) if last_valid.exists() && active_path != last_valid => {
                std::fs::read_to_string(&last_valid).map_err(|e| {
                    ForgeError::Internal(format!("Failed to read fallback restore journal: {}", e))
                })?
            }
            Err(e) => {
                return Err(ForgeError::Internal(format!(
                    "Failed to read restore journal: {}",
                    e
                )));
            }
        };

        let journal: Self = match serde_json::from_str(&content) {
            Ok(j) => j,
            Err(e) if last_valid.exists() && active_path != last_valid => {
                if let Ok(c) = std::fs::read_to_string(&last_valid) {
                    serde_json::from_str(&c).map_err(|_| {
                        ForgeError::Internal(format!("Corrupt restore journal: {}", e))
                    })?
                } else {
                    return Err(ForgeError::Internal(format!(
                        "Corrupt restore journal: {}",
                        e
                    )));
                }
            }
            Err(e) => {
                return Err(ForgeError::Internal(format!(
                    "Corrupt restore journal: {}",
                    e
                )))
            }
        };

        Ok(Some(journal))
    }

    /// Persist the updated state machine phase to disk with sync_all and atomic replacement.
    /// Preserves last_valid backup to ensure power-loss safety and non-destructive roll forward.
    pub fn persist(&mut self, phase: RestorePhase) -> Result<(), ForgeError> {
        use std::io::Write;
        self.phase = phase.clone();
        if let RestorePhase::CommitDecision(ref cdr) = phase {
            self.commit_decision = Some(cdr.clone());
        }
        self.updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let path = Self::journal_path(&self.target_storage_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        let _ = std::fs::create_dir_all(parent);

        // Preserve previous valid journal as last_valid backup
        let last_valid = Self::last_valid_journal_path(&self.target_storage_root);
        if path.exists() {
            let _ = std::fs::copy(&path, &last_valid);
        }

        let tmp_path = parent.join(format!(".tmp_jnl_{}.json", uuid::Uuid::new_v4().simple()));
        let json_data = serde_json::to_string_pretty(self)
            .map_err(|e| ForgeError::Internal(format!("Failed to serialize journal: {}", e)))?;
        {
            let mut f = std::fs::File::create(&tmp_path).map_err(|e| {
                ForgeError::Internal(format!("Failed to create temporary journal: {}", e))
            })?;
            f.write_all(json_data.as_bytes()).map_err(|e| {
                ForgeError::Internal(format!("Failed to write temporary journal: {}", e))
            })?;
            f.flush().map_err(|e| ForgeError::Internal(e.to_string()))?;
            f.sync_all()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        super::safe_atomic_replace(&tmp_path, &path)?;
        Ok(())
    }

    /// Remove the journal file upon successful completion.
    pub fn remove(target_storage_root: &Path) -> Result<(), ForgeError> {
        let path = Self::journal_path(target_storage_root);
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
        let last_valid = Self::last_valid_journal_path(target_storage_root);
        if last_valid.exists() {
            let _ = std::fs::remove_file(&last_valid);
        }
        let legacy = Self::legacy_journal_path(target_storage_root);
        if legacy.exists() {
            let _ = std::fs::remove_file(&legacy);
        }
        let control_dir = target_storage_root.join(Self::CONTROL_DIR_NAME);
        if control_dir.exists() {
            let _ = std::fs::remove_dir_all(&control_dir);
        }
        Ok(())
    }

    /// Synchronous journal recovery.
    /// Cleans up uncommitted staging artifacts if interrupted before global commit decision.
    /// Safely forward-recovers committed restores across all local storage participants; fails closed if unrecoverable.
    pub fn handle_startup_recovery_sync(storage_root: &Path) -> Result<Option<String>, ForgeError> {
        if let Some(mut journal) = Self::load(storage_root)? {
            match journal.phase {
                RestorePhase::Committed | RestorePhase::Completed => {
                    let old_dir =
                        storage_root.join(format!(".old_generation_{}", journal.restore_id));
                    if old_dir.exists() {
                        let _ = std::fs::remove_dir_all(&old_dir);
                    }
                    let _ = Self::remove(storage_root);
                    Ok(None)
                }
                RestorePhase::Initialized
                | RestorePhase::StagedStorage
                | RestorePhase::StagedDb
                | RestorePhase::Prepared
                | RestorePhase::RolledBack(_)
                | RestorePhase::Aborted(_) => {
                    tracing::warn!(
                        "Detected interrupted restore session '{}' before global commit decision in phase '{:?}'. Cleaning up staging directory {:?}...",
                        journal.restore_id,
                        journal.phase,
                        journal.staging_storage_root
                    );
                    if journal.staging_storage_root.exists() {
                        let _ = std::fs::remove_dir_all(&journal.staging_storage_root);
                    }
                    let _ = Self::remove(storage_root);
                    Ok(Some(format!(
                        "Interrupted restore '{}' before global commit decision was cleanly rolled back.",
                        journal.restore_id
                    )))
                }
                RestorePhase::CommitDecision(_)
                | RestorePhase::Publishing { .. }
                | RestorePhase::DbPublished
                | RestorePhase::DatabaseRestored
                | RestorePhase::StoragePublished
                | RestorePhase::Published
                | RestorePhase::Verified => {
                    // Global Commit Decision was reached! Irreversible forward recovery.
                    tracing::info!(
                        "Forward recovering committed restore '{}' in phase '{:?}' to target generation '{}'.",
                        journal.restore_id,
                        journal.phase,
                        journal.target_generation
                    );

                    // 1. Resume / Complete live storage publication if staging exists
                    let storage_gen = Self::read_storage_generation(storage_root);
                    let storage_needs_cutover =
                        storage_gen.as_deref() != Some(&journal.target_generation);

                    if storage_needs_cutover && journal.staging_storage_root.exists() {
                        let old_dir =
                            storage_root.join(format!(".old_generation_{}", journal.restore_id));
                        let _ = std::fs::create_dir_all(&old_dir);

                        // Move existing entries to old_generation
                        if let Ok(entries) = std::fs::read_dir(storage_root) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                if p == old_dir || p == journal.staging_storage_root {
                                    continue;
                                }
                                let name = entry.file_name().to_string_lossy().to_string();
                                if !name.starts_with(".old_generation_")
                                    && !name.starts_with(".staged_generation_")
                                    && !name.starts_with(".tmp")
                                    && !name.starts_with(".spine_restore_control")
                                    && name != Self::JOURNAL_FILE_NAME
                                {
                                    let dst = old_dir.join(&name);
                                    let _ = super::safe_move_file(&p, &dst);
                                }
                            }
                        }

                        // Move staging entries to storage root
                        if let Ok(entries) = std::fs::read_dir(&journal.staging_storage_root) {
                            for entry in entries.flatten() {
                                let p = entry.path();
                                let name = entry.file_name().to_string_lossy().to_string();
                                if name != "database"
                                    && name != "staged_ssh"
                                    && !name.starts_with(".tmp")
                                {
                                    let dst = storage_root.join(&name);
                                    let _ = super::safe_move_file(&p, &dst);
                                }
                            }
                        }

                        Self::write_storage_generation(storage_root, &journal.target_generation)?;
                    }

                    // 2. Resume / Complete SSH identity cutover if staged
                    let staged_ssh = journal.staging_storage_root.join("staged_ssh");
                    let ssh_dir = storage_root.join(".ssh");
                    if staged_ssh.exists() && ssh_dir.exists() {
                        if let Ok(entries) = std::fs::read_dir(&staged_ssh) {
                            for entry in entries.flatten() {
                                let dst = ssh_dir.join(entry.file_name());
                                let _ = std::fs::copy(entry.path(), &dst);
                            }
                        }
                    }

                    // 3. Verify storage generation now matches
                    let updated_storage_gen = Self::read_storage_generation(storage_root);
                    if updated_storage_gen.as_deref() == Some(&journal.target_generation) {
                        let old_dir =
                            storage_root.join(format!(".old_generation_{}", journal.restore_id));
                        if old_dir.exists() {
                            let _ = std::fs::remove_dir_all(&old_dir);
                        }
                        if journal.staging_storage_root.exists() {
                            let _ = std::fs::remove_dir_all(&journal.staging_storage_root);
                        }
                        journal.persist(RestorePhase::Committed)?;
                        let _ = Self::remove(storage_root);
                        Ok(Some(format!(
                            "Interrupted restore '{}' at '{:?}' was forward-recovered to Committed and rolled forward to target generation '{}'.",
                            journal.restore_id,
                            journal.phase,
                            journal.target_generation
                        )))
                    } else {
                        Err(ForgeError::Internal(format!(
                            "FATAL STARTUP BLOCKED: Committed restore '{}' in phase '{:?}' could not complete storage cutover. Staging directory may be missing.",
                            journal.restore_id,
                            journal.phase
                        )))
                    }
                }
                RestorePhase::Failed(ref reason) => {
                    if journal.commit_decision.is_none() {
                        // Failed before commit decision -> clean up staging and abort
                        if journal.staging_storage_root.exists() {
                            let _ = std::fs::remove_dir_all(&journal.staging_storage_root);
                        }
                        let _ = Self::remove(storage_root);
                        Ok(Some(format!(
                            "Interrupted restore '{}' previously failed before decision and was cleaned up: {}",
                            journal.restore_id, reason
                        )))
                    } else {
                        Err(ForgeError::Internal(format!(
                            "FATAL STARTUP BLOCKED: Restore session '{}' with committed decision failed with: {}. Manual intervention required.",
                            journal.restore_id,
                            reason
                        )))
                    }
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
