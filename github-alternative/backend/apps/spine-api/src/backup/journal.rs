use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Durable preparation acknowledgement for an authoritative restore participant.
pub use spine_cluster::protocol::ParticipantAcknowledgement;

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

/// Durable per-participant recovery, convergence, and finalization state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ParticipantRecoveryState {
    pub participant_id: String,
    #[serde(default)]
    pub snapshot_id: Option<String>,
    #[serde(default)]
    pub converged: bool,
    #[serde(default)]
    pub finalized: bool,
    #[serde(default)]
    pub verified_generation: Option<String>,
    #[serde(default)]
    pub updated_at_nanos: i64,
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
    pub participant_progress: BTreeMap<String, ParticipantRecoveryState>,
    #[serde(default)]
    pub commit_decision: Option<CommitDecisionRecord>,
    #[serde(default)]
    pub sequence_number: u64,
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
            participant_progress: BTreeMap::new(),
            commit_decision: None,
            sequence_number: 1,
        }
    }

    /// Parse numeric cluster fencing generation with checked bounds (>= 1, valid u64).
    /// Fails closed on empty, zero, malformed, or overflowing strings.
    pub fn parse_generation_u64(gen_str: &str) -> Result<u64, ForgeError> {
        let trimmed = gen_str.trim();
        if trimmed.is_empty() {
            return Err(ForgeError::Internal(
                "FATAL RESTORE RECOVERY FAILURE: Target generation token is empty".into(),
            ));
        }
        let parsed = trimmed.parse::<u64>().map_err(|e| {
            ForgeError::Internal(format!(
                "FATAL RESTORE RECOVERY FAILURE: Malformed or overflowing generation token '{}': {}",
                trimmed, e
            ))
        })?;
        if parsed == 0 {
            return Err(ForgeError::Internal(
                "FATAL RESTORE RECOVERY FAILURE: Numeric fencing generation '0' is invalid (must be >= 1)".into(),
            ));
        }
        Ok(parsed)
    }

    /// Durably record the snapshot ID bound to a participant before CommitDecision.
    pub fn record_participant_snapshot(
        &mut self,
        participant_id: &str,
        snapshot_id: &str,
    ) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let entry = self
            .participant_progress
            .entry(participant_id.to_string())
            .or_insert_with(|| ParticipantRecoveryState {
                participant_id: participant_id.to_string(),
                snapshot_id: None,
                converged: false,
                finalized: false,
                verified_generation: None,
                updated_at_nanos: now,
            });
        entry.snapshot_id = Some(snapshot_id.to_string());
        entry.updated_at_nanos = now;
        self.persist(self.phase.clone())
    }

    /// Update and persist per-participant recovery convergence & finalization progress durably.
    pub fn record_participant_recovery(
        &mut self,
        participant_id: &str,
        converged: bool,
        finalized: bool,
        verified_generation: Option<&str>,
    ) -> Result<(), ForgeError> {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let existing_snap = self
            .participant_progress
            .get(participant_id)
            .and_then(|s| s.snapshot_id.clone());
        let state = ParticipantRecoveryState {
            participant_id: participant_id.to_string(),
            snapshot_id: existing_snap,
            converged,
            finalized,
            verified_generation: verified_generation.map(|s| s.to_string()),
            updated_at_nanos: now,
        };
        let next_phase = self.phase.clone();
        self.participant_progress
            .insert(participant_id.to_string(), state);
        self.persist(next_phase)
    }

    /// Resolve exact staged/committed snapshot ID for a cluster participant across restarts.
    pub fn snapshot_id_for_participant(&self, participant_id: &str) -> Option<String> {
        if let Some(snap) = self
            .participant_progress
            .get(participant_id)
            .and_then(|s| s.snapshot_id.clone())
        {
            if !snap.trim().is_empty() {
                return Some(snap);
            }
        }
        if let Some(ack) = self.acknowledgements.get(participant_id) {
            let rec_id = &ack.durable_staging_record_id;
            if let Some(rest) = rec_id.strip_prefix(".staged_coordinator_restore_") {
                if let Some(snap) = rest.strip_suffix(".json") {
                    if !snap.trim().is_empty() {
                        return Some(snap.to_string());
                    }
                }
            }
            if let Some(rest) = rec_id.strip_prefix(".staged_volume_") {
                if let Some((snap, _)) = rest.split_once('/') {
                    if !snap.trim().is_empty() {
                        return Some(snap.to_string());
                    }
                }
            }
        }
        None
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
        let insert_sql = r#"
            INSERT INTO _restore_generation (id, generation_token, backup_id, restored_at_nanos, status)
            VALUES (1, $1, $2, $3, 'COMMITTED')
            ON CONFLICT (id) DO UPDATE SET
                generation_token = EXCLUDED.generation_token,
                backup_id = EXCLUDED.backup_id,
                restored_at_nanos = EXCLUDED.restored_at_nanos,
                status = EXCLUDED.status
        "#;
        if let Err(e) = sqlx::query(insert_sql)
            .bind(generation)
            .bind(backup_id)
            .bind(now_nanos)
            .execute(&mut **tx)
            .await
        {
            // Fallback if table does not exist: attempt creation and retry insert
            let create_sql = r#"
                CREATE TABLE IF NOT EXISTS _restore_generation (
                    id INT PRIMARY KEY DEFAULT 1,
                    generation_token VARCHAR(128) NOT NULL,
                    backup_id VARCHAR(128) NOT NULL,
                    restored_at_nanos BIGINT NOT NULL,
                    status VARCHAR(32) NOT NULL DEFAULT 'COMMITTED'
                )
            "#;
            let _ = sqlx::query(create_sql).execute(&mut **tx).await;

            sqlx::query(insert_sql)
                .bind(generation)
                .bind(backup_id)
                .bind(now_nanos)
                .execute(&mut **tx)
                .await
                .map_err(|e2| {
                    ForgeError::Internal(format!(
                        "Failed to record database restore generation in transaction (attempt 1: {}, attempt 2: {})",
                        e, e2
                    ))
                })?;
        }
        Ok(())
    }

    /// Record generation token in target database.
    pub async fn record_db_generation(
        pool: &sqlx::PgPool,
        generation: &str,
        backup_id: &str,
    ) -> Result<(), ForgeError> {
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let insert_sql = r#"
            INSERT INTO _restore_generation (id, generation_token, backup_id, restored_at_nanos, status)
            VALUES (1, $1, $2, $3, 'COMMITTED')
            ON CONFLICT (id) DO UPDATE SET
                generation_token = EXCLUDED.generation_token,
                backup_id = EXCLUDED.backup_id,
                restored_at_nanos = EXCLUDED.restored_at_nanos,
                status = EXCLUDED.status
        "#;

        if let Err(e) = sqlx::query(insert_sql)
            .bind(generation)
            .bind(backup_id)
            .bind(now_nanos)
            .execute(pool)
            .await
        {
            // Fallback for fresh initialization if table does not yet exist: attempt creation and retry insert
            let create_sql = r#"
                CREATE TABLE IF NOT EXISTS _restore_generation (
                    id INT PRIMARY KEY DEFAULT 1,
                    generation_token VARCHAR(128) NOT NULL,
                    backup_id VARCHAR(128) NOT NULL,
                    restored_at_nanos BIGINT NOT NULL,
                    status VARCHAR(32) NOT NULL DEFAULT 'COMMITTED'
                )
            "#;
            let _ = sqlx::query(create_sql).execute(pool).await;

            sqlx::query(insert_sql)
                .bind(generation)
                .bind(backup_id)
                .bind(now_nanos)
                .execute(pool)
                .await
                .map_err(|e2| {
                    ForgeError::Internal(format!(
                        "Failed to record database restore generation: initial error: {}, retry error: {}",
                        e, e2
                    ))
                })?;
        }
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
        // 1. Validate bound restore ID (strict equality)
        if ack.restore_id != self.restore_id {
            return Err(ForgeError::Internal(format!(
                "Acknowledgement restore_id mismatch: expected '{}', got '{}'",
                self.restore_id, ack.restore_id
            )));
        }
        // 2. Validate bound backup ID (strict equality)
        if ack.backup_id != self.backup_id {
            return Err(ForgeError::Internal(format!(
                "Acknowledgement backup_id mismatch: expected '{}', got '{}'",
                self.backup_id, ack.backup_id
            )));
        }
        // 3. Validate target generation (strict equality)
        if ack.target_generation != self.target_generation {
            return Err(ForgeError::Internal(format!(
                "Acknowledgement target_generation mismatch: expected '{}', got '{}'",
                self.target_generation, ack.target_generation
            )));
        }
        // 4. Validate prepared state
        if !ack.prepared {
            return Err(ForgeError::Internal(format!(
                "Participant '{}' acknowledgement indicates preparation failed (prepared=false)",
                ack.participant_id
            )));
        }
        // 5. Validate non-empty, exactly 64-hex SHA-256 payload digest
        if ack.staged_digest.len() != 64
            || !ack.staged_digest.chars().all(|c| c.is_ascii_hexdigit())
        {
            return Err(ForgeError::Internal(format!(
                "Participant '{}' acknowledgement contains invalid SHA-256 digest: '{}' (must be exactly 64 hex characters)",
                ack.participant_id, ack.staged_digest
            )));
        }
        // 6. Validate canonical resource ID and durable staging record ID
        if ack.canonical_resource_id.trim().is_empty() {
            return Err(ForgeError::Internal(format!(
                "Participant '{}' acknowledgement missing canonical resource identity",
                ack.participant_id
            )));
        }
        if ack.durable_staging_record_id.trim().is_empty() {
            return Err(ForgeError::Internal(format!(
                "Participant '{}' acknowledgement missing durable staging record identity",
                ack.participant_id
            )));
        }

        // 7. Reject contradictory duplicate acknowledgement
        if let Some(existing) = self.acknowledgements.get(&ack.participant_id) {
            if existing.staged_digest != ack.staged_digest
                || existing.durable_staging_record_id != ack.durable_staging_record_id
            {
                return Err(ForgeError::Internal(format!(
                    "Contradictory duplicate acknowledgement for participant '{}'",
                    ack.participant_id
                )));
            }
        }

        self.acknowledgements
            .insert(ack.participant_id.clone(), ack);
        self.persist(self.phase.clone())
    }

    /// Check if all required participants have valid preparation acknowledgements.
    pub fn has_all_required_acks(&self, required_participants: &[String]) -> bool {
        for p in required_participants {
            match self.acknowledgements.get(p) {
                Some(ack)
                    if ack.prepared
                        && !ack.staged_digest.is_empty()
                        && !ack.canonical_resource_id.is_empty()
                        && !ack.durable_staging_record_id.is_empty() =>
                {
                    continue
                }
                _ => return false,
            }
        }
        true
    }

    /// Strict commit-decision gate validator: rejects missing, duplicate, stale or unverified participants.
    pub fn validate_commit_decision_gate(
        &self,
        expected_participants: &[String],
    ) -> Result<(), ForgeError> {
        for exp in expected_participants {
            match self.acknowledgements.get(exp) {
                Some(ack) => {
                    if ack.target_generation != self.target_generation {
                        return Err(ForgeError::Internal(format!(
                            "Participant '{}' target generation mismatch: expected {}, got {}",
                            exp, self.target_generation, ack.target_generation
                        )));
                    }
                    if !ack.prepared {
                        return Err(ForgeError::Internal(format!(
                            "Participant '{}' acknowledgement is not prepared",
                            exp
                        )));
                    }
                    if ack.staged_digest.trim().is_empty() {
                        return Err(ForgeError::Internal(format!(
                            "Participant '{}' acknowledgement has empty digest",
                            exp
                        )));
                    }
                    if ack.canonical_resource_id.trim().is_empty() {
                        return Err(ForgeError::Internal(format!(
                            "Participant '{}' missing canonical resource ID",
                            exp
                        )));
                    }
                    if ack.durable_staging_record_id.trim().is_empty() {
                        return Err(ForgeError::Internal(format!(
                            "Participant '{}' missing durable staging record ID",
                            exp
                        )));
                    }
                }
                None => {
                    return Err(ForgeError::Internal(format!(
                        "Commit decision gate rejected: missing required preparation acknowledgement for participant '{}'",
                        exp
                    )));
                }
            }
        }
        if self.acknowledgements.len() < expected_participants.len() {
            return Err(ForgeError::Internal(format!(
                "Commit decision gate rejected: required {} participants, found {}",
                expected_participants.len(),
                self.acknowledgements.len()
            )));
        }
        Ok(())
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
    /// In-memory state is strictly mutated ONLY after durable disk persistence succeeds.
    pub fn persist(&mut self, next_phase: RestorePhase) -> Result<(), ForgeError> {
        use std::io::Write;

        // 1. Construct candidate next state independently
        let mut candidate = self.clone();
        candidate.phase = next_phase.clone();
        if let RestorePhase::CommitDecision(ref cdr) = next_phase {
            candidate.commit_decision = Some(cdr.clone());
        }
        candidate.sequence_number += 1;
        candidate.updated_at_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let path = Self::journal_path(&candidate.target_storage_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;

        // 2. Preserve previous valid journal as last_valid backup
        let last_valid = Self::last_valid_journal_path(&candidate.target_storage_root);
        if path.exists() {
            let existing_bytes = std::fs::read(&path).map_err(|e| {
                ForgeError::Internal(format!("Failed to read existing journal for backup: {}", e))
            })?;
            let existing: Self = serde_json::from_slice(&existing_bytes).map_err(|e| {
                ForgeError::Internal(format!(
                    "Existing journal is corrupt, cannot create last_valid backup: {}",
                    e
                ))
            })?;

            if existing.restore_id == candidate.restore_id {
                // Sequence monotonicity check
                if candidate.sequence_number <= existing.sequence_number {
                    return Err(ForgeError::Internal(format!(
                        "Restore journal sequence monotonicity violation: candidate sequence {} <= existing sequence {}",
                        candidate.sequence_number, existing.sequence_number
                    )));
                }
                // Once CommitDecision is made, subsequent transitions MUST retain it
                if existing.commit_decision.is_some() && candidate.commit_decision.is_none() {
                    return Err(ForgeError::Internal(
                        "Restore journal invariant violation: cannot erase durable commit_decision once established"
                            .to_string(),
                    ));
                }
            } else if existing.commit_decision.is_some()
                && existing.phase != RestorePhase::Completed
            {
                // Prevent starting or persisting a different restore over an unrecovered committed restore
                return Err(ForgeError::Internal(format!(
                    "Cannot overwrite unrecovered committed restore session '{}' with new restore '{}'",
                    existing.restore_id, candidate.restore_id
                )));
            }

            let tmp_lv = parent.join(format!(".tmp_lv_{}.json", uuid::Uuid::new_v4().simple()));
            let mut lv_file = std::fs::File::create(&tmp_lv).map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to create temporary last_valid journal: {}",
                    e
                ))
            })?;
            lv_file
                .write_all(&existing_bytes)
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            lv_file
                .flush()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            lv_file
                .sync_all()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            super::safe_atomic_replace(&tmp_lv, &last_valid)?;
        }

        // 3. Serialize candidate, pre-validate deserialization, and write to temporary file
        let tmp_path = parent.join(format!(".tmp_jnl_{}.json", uuid::Uuid::new_v4().simple()));
        let json_data = serde_json::to_string_pretty(&candidate)
            .map_err(|e| ForgeError::Internal(format!("Failed to serialize journal: {}", e)))?;
        serde_json::from_str::<Self>(&json_data).map_err(|e| {
            ForgeError::Internal(format!(
                "Candidate journal serialization validation failed: {}",
                e
            ))
        })?;
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

        // 4. Atomic replacement to primary journal path
        super::safe_atomic_replace(&tmp_path, &path)?;

        // 5. Readback verification of primary journal across ALL fields
        let readback_content = std::fs::read_to_string(&path).map_err(|e| {
            ForgeError::Internal(format!("Journal readback failed after persist: {}", e))
        })?;
        let readback_journal = serde_json::from_str::<Self>(&readback_content).map_err(|e| {
            ForgeError::Internal(format!("Journal readback verification failed: {}", e))
        })?;

        if readback_journal.restore_id != candidate.restore_id
            || readback_journal.backup_id != candidate.backup_id
            || readback_journal.target_generation != candidate.target_generation
            || readback_journal.sequence_number != candidate.sequence_number
            || readback_journal.phase != candidate.phase
            || readback_journal.acknowledgements.len() != candidate.acknowledgements.len()
            || readback_journal.participant_progress != candidate.participant_progress
            || readback_journal.published_participants != candidate.published_participants
            || readback_journal.commit_decision != candidate.commit_decision
        {
            return Err(ForgeError::Internal(
                "Journal readback mismatch: persisted record does not match candidate across all fields".into(),
            ));
        }

        // 6. Synchronize directory
        super::sync_dir(parent)?;

        // 7. ONLY AFTER successful persistence: mutate in-memory state
        *self = candidate;
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
    /// Forward-recovers local storage & SSH while strictly RETAINING the durable journal for Committed and all post-decision phases
    /// until full cluster convergence and finalization are confirmed.
    pub fn handle_startup_recovery_sync(storage_root: &Path) -> Result<Option<String>, ForgeError> {
        if let Some(journal) = Self::load(storage_root)? {
            match journal.phase {
                RestorePhase::Completed => {
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
                | RestorePhase::Verified
                | RestorePhase::Committed => {
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
                                    let _ = super::safe_move_path(&p, &dst);
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
                                    let _ = super::safe_move_path(&p, &dst);
                                }
                            }
                        }

                        Self::write_storage_generation(storage_root, &journal.target_generation)?;
                    }

                    // 2. Resume / Complete SSH identity cutover if staged
                    let staged_ssh = journal.staging_storage_root.join("staged_ssh");
                    let ssh_dir = crate::ssh::host_key::resolve_ssh_dir(storage_root);
                    if staged_ssh.exists() {
                        std::fs::create_dir_all(&ssh_dir)
                            .map_err(|e| ForgeError::Internal(e.to_string()))?;
                        if let Ok(entries) = std::fs::read_dir(&staged_ssh) {
                            for entry in entries.flatten() {
                                let dst = ssh_dir.join(entry.file_name());
                                std::fs::copy(entry.path(), &dst).map_err(|e| {
                                    ForgeError::Internal(format!(
                                        "Failed to forward-recover staged SSH file {}: {}",
                                        dst.display(),
                                        e
                                    ))
                                })?;
                            }
                        }
                    }

                    // 3. Verify storage generation now matches
                    let updated_storage_gen = Self::read_storage_generation(storage_root);
                    if updated_storage_gen.as_deref() != Some(&journal.target_generation) {
                        return Err(ForgeError::Internal(format!(
                            "FATAL STARTUP BLOCKED: Committed restore '{}' in phase '{:?}' could not complete storage cutover. Staging directory may be missing.",
                            journal.restore_id,
                            journal.phase
                        )));
                    }

                    if journal.staging_storage_root.exists() {
                        let _ = std::fs::remove_dir_all(&journal.staging_storage_root);
                    }

                    // Preserve journal and commit_decision across ALL post-decision phases (including Committed)
                    // until handle_startup_recovery_cluster verifies and finalizes every required participant.
                    Ok(Some(format!(
                        "Interrupted restore '{}' at '{:?}' forward-recovered local storage and rolled forward to target generation '{}'; journal retained until global convergence.",
                        journal.restore_id,
                        journal.phase,
                        journal.target_generation
                    )))
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

    /// Database generation verifier for cluster recovery.
    pub async fn handle_startup_recovery_cluster_with_db_check<F>(
        storage_root: &Path,
        db_check: Option<F>,
        pool: Option<&sqlx::PgPool>,
        coordinator: Option<&crate::CoordinatorClient>,
        storage_nodes: &[(String, String)],
    ) -> Result<Option<String>, ForgeError>
    where
        F: Fn(&str) -> Result<Option<String>, ForgeError> + Send + Sync,
    {
        let sync_msg = Self::handle_startup_recovery_sync(storage_root)?;

        let journal_opt = Self::load(storage_root)?;
        let mut reconciled_storage_gen: Option<String> =
            Self::read_storage_generation(storage_root);

        if let Some(mut journal) = journal_opt {
            let target_gen = journal.target_generation.clone();
            let decision = journal.commit_decision.clone().ok_or_else(|| {
                ForgeError::Internal(format!(
                    "FATAL RESTORE RECOVERY FAILURE: Post-decision restore journal '{}' in phase '{:?}' is missing durable CommitDecisionRecord",
                    journal.restore_id, journal.phase
                ))
            })?;

            if decision.restore_id != journal.restore_id {
                return Err(ForgeError::Internal(format!(
                    "FATAL RESTORE RECOVERY FAILURE: CommitDecision restore_id '{}' != journal restore_id '{}'",
                    decision.restore_id, journal.restore_id
                )));
            }
            if decision.new_generation != target_gen {
                return Err(ForgeError::Internal(format!(
                    "FATAL RESTORE RECOVERY FAILURE: CommitDecision new_generation '{}' != journal target_generation '{}'",
                    decision.new_generation, target_gen
                )));
            }
            if decision.required_participants.is_empty() {
                return Err(ForgeError::Internal(
                    "FATAL RESTORE RECOVERY FAILURE: CommitDecision has empty required_participants"
                        .into(),
                ));
            }

            let canonical_role = |p_id: &str| -> Result<&'static str, ForgeError> {
                match p_id {
                    "postgres_data" => Ok("postgres_data"),
                    "spine_storage" => Ok("spine_storage"),
                    "spine_ssh" => Ok("spine_ssh"),
                    "coordinator" | "coordinator_data" => Ok("coordinator"),
                    "node-1" | "storage_1_data" => Ok("node-1"),
                    "node-2" | "storage_2_data" => Ok("node-2"),
                    "node-3" | "storage_3_data" => Ok("node-3"),
                    other => Err(ForgeError::Internal(format!(
                        "FATAL RESTORE RECOVERY FAILURE: Unknown required participant '{}' in CommitDecisionRecord",
                        other
                    ))),
                }
            };

            let mut needs_numeric_gen = coordinator.is_some() || !storage_nodes.is_empty();
            for p_id in &decision.required_participants {
                let role = canonical_role(p_id)?;
                if role == "coordinator" || role.starts_with("node-") {
                    needs_numeric_gen = true;
                }
            }

            // Requirement 3: Parse target generation ONCE using checked conversion BEFORE making RPCs or mutating state
            let expected_num_gen = if needs_numeric_gen {
                Some(Self::parse_generation_u64(&target_gen)?)
            } else {
                None
            };

            let required_participants = decision.required_participants.clone();

            for raw_p_id in &required_participants {
                let role = canonical_role(raw_p_id)?;
                let prev_progress = journal
                    .participant_progress
                    .get(raw_p_id)
                    .cloned()
                    .unwrap_or_default();

                match role {
                    "postgres_data" => {
                        let db_gen_opt = if let Some(ref check_fn) = db_check {
                            check_fn(&target_gen)?
                        } else if let Some(pool) = pool {
                            let row: Result<Option<(String,)>, _> = sqlx::query_as(
                                "SELECT generation_token FROM _restore_generation WHERE id = 1",
                            )
                            .fetch_optional(pool)
                            .await;
                            row.map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Failed to query PostgreSQL _restore_generation: {}",
                                    e
                                ))
                            })?
                            .map(|r| r.0)
                        } else {
                            return Err(ForgeError::Internal(
                                "FATAL RESTORE RECOVERY FAILURE: Required participant 'postgres_data' cannot be verified because PostgreSQL pool is not connected"
                                    .into(),
                            ));
                        };

                        match db_gen_opt {
                            Some(ref db_gen) if db_gen == &target_gen => {
                                journal.record_participant_recovery(
                                    raw_p_id,
                                    true,
                                    true,
                                    Some(&target_gen),
                                )?;
                            }
                            Some(db_gen) => {
                                return Err(ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: PostgreSQL generation '{}' != target generation '{}'",
                                    db_gen, target_gen
                                )));
                            }
                            None => {
                                return Err(ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: PostgreSQL missing _restore_generation record during recovery of restore '{}'",
                                    journal.restore_id
                                )));
                            }
                        }
                    }
                    "spine_storage" => {
                        let s_gen = Self::read_storage_generation(storage_root);
                        if s_gen.as_deref() != Some(&target_gen) {
                            return Err(ForgeError::Internal(format!(
                                "FATAL RESTORE RECOVERY FAILURE: Storage generation '{:?}' != target generation '{}'",
                                s_gen, target_gen
                            )));
                        }
                        // Mark converged now; finalized happens once old_generation is cleaned up after all participants converge
                        journal.record_participant_recovery(
                            raw_p_id,
                            true,
                            prev_progress.finalized,
                            Some(&target_gen),
                        )?;
                    }
                    "spine_ssh" => {
                        let ssh_dir = crate::ssh::host_key::resolve_ssh_dir(storage_root);
                        let host_key = ssh_dir.join("host_key_ed25519");
                        if !host_key.exists() {
                            return Err(ForgeError::Internal(format!(
                                "FATAL RESTORE RECOVERY FAILURE: Required SSH host key '{}' missing after restore",
                                host_key.display()
                            )));
                        }
                        let old_ssh = storage_root.join(format!(".old_ssh_{}", journal.restore_id));
                        if old_ssh.exists() {
                            std::fs::remove_dir_all(&old_ssh).map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Failed to finalize old SSH directory {}: {}",
                                    old_ssh.display(),
                                    e
                                ))
                            })?;
                        }
                        journal.record_participant_recovery(
                            raw_p_id,
                            true,
                            true,
                            Some(&target_gen),
                        )?;
                    }
                    "coordinator" => {
                        let coord = coordinator.ok_or_else(|| {
                            ForgeError::Internal(format!(
                                "FATAL RESTORE RECOVERY FAILURE: Required participant '{}' missing coordinator client configuration",
                                raw_p_id
                            ))
                        })?;
                        let expected_gen = expected_num_gen.expect("validated up front");
                        let (in_maint, coord_gen, _, responding_id) =
                            coord.query_node_details().await.map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Failed to query coordinator details: {}",
                                    e
                                ))
                            })?;

                        if responding_id.as_deref() != Some("coordinator") {
                            return Err(ForgeError::Internal(format!(
                                "FATAL RESTORE RECOVERY FAILURE: Coordinator endpoint at {} responded with identity {:?}, expected 'coordinator'",
                                coord.addr(),
                                responding_id
                            )));
                        }

                        let snapshot_id = journal
                            .snapshot_id_for_participant(raw_p_id)
                            .or_else(|| journal.snapshot_id_for_participant("coordinator"))
                            .unwrap_or_else(|| format!("rst_{}", journal.restore_id));

                        if coord_gen != expected_gen || in_maint {
                            match coord.query_staged_restore(&snapshot_id).await {
                                Ok(Some(ack)) => {
                                    if ack.participant_id != "coordinator" {
                                        return Err(ForgeError::Internal(format!(
                                            "FATAL RESTORE RECOVERY FAILURE: Staged coordinator acknowledgement identity '{}' != 'coordinator'",
                                            ack.participant_id
                                        )));
                                    }
                                    if !in_maint {
                                        coord.enter_maintenance(expected_gen).await.map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed to enter maintenance on coordinator before commit: {}",
                                                e
                                            ))
                                        })?;
                                    }
                                    coord
                                        .commit_restore(&snapshot_id, expected_gen)
                                        .await
                                        .map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed to commit staged restore '{}' on coordinator: {}",
                                                snapshot_id, e
                                            ))
                                        })?;

                                    let (post_maint, post_gen, _, post_id) =
                                        coord.query_node_details().await.map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed post-commit verification query on coordinator: {}",
                                                e
                                            ))
                                        })?;
                                    if post_id.as_deref() != Some("coordinator")
                                        || post_maint
                                        || post_gen != expected_gen
                                    {
                                        return Err(ForgeError::Internal(format!(
                                            "FATAL RESTORE RECOVERY FAILURE: Coordinator post-commit state invalid (id={:?}, maint={}, gen={}, expected={})",
                                            post_id, post_maint, post_gen, expected_gen
                                        )));
                                    }
                                }
                                Ok(None) => {
                                    return Err(ForgeError::Internal(format!(
                                        "FATAL RESTORE RECOVERY FAILURE: Coordinator generation ({}) != target ({}) (in_maint={}) and no staged restore '{}'",
                                        coord_gen, expected_gen, in_maint, snapshot_id
                                    )));
                                }
                                Err(e) => {
                                    return Err(ForgeError::Internal(format!(
                                        "FATAL RESTORE RECOVERY FAILURE: Failed querying staged restore '{}' on coordinator: {}",
                                        snapshot_id, e
                                    )));
                                }
                            }
                        }

                        journal.record_participant_recovery(
                            raw_p_id,
                            true,
                            prev_progress.finalized,
                            Some(&target_gen),
                        )?;

                        if !prev_progress.finalized {
                            coord.finalize_restore(&snapshot_id).await.map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Coordinator finalize_restore('{}') failed: {}",
                                    snapshot_id, e
                                ))
                            })?;
                            journal.record_participant_recovery(
                                raw_p_id,
                                true,
                                true,
                                Some(&target_gen),
                            )?;
                        }
                    }
                    expected_node_id if expected_node_id.starts_with("node-") => {
                        let addr = storage_nodes
                            .iter()
                            .find(|(id, _)| id == expected_node_id)
                            .map(|(_, a)| a)
                            .ok_or_else(|| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Required storage node '{}' (participant '{}') missing from configured storage_nodes endpoints",
                                    expected_node_id, raw_p_id
                                ))
                            })?;

                        let expected_gen = expected_num_gen.expect("validated up front");
                        let client = crate::ClusterNodeClient::new(addr.clone());
                        let (in_maint, node_gen, _, responding_id) =
                            client.query_node_details().await.map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Failed to query storage node '{}' at {}: {}",
                                    expected_node_id, addr, e
                                ))
                            })?;

                        if responding_id.as_deref() != Some(expected_node_id) {
                            return Err(ForgeError::Internal(format!(
                                "FATAL RESTORE RECOVERY FAILURE: Storage node at {} responded with ID {:?}, expected '{}'",
                                addr, responding_id, expected_node_id
                            )));
                        }

                        let snapshot_id = journal
                            .snapshot_id_for_participant(raw_p_id)
                            .or_else(|| journal.snapshot_id_for_participant(expected_node_id))
                            .unwrap_or_else(|| format!("rst_{}", journal.restore_id));

                        if node_gen != expected_gen || in_maint {
                            match client.query_staged_restore(&snapshot_id).await {
                                Ok(Some(ack)) => {
                                    if ack.participant_id != expected_node_id
                                        && ack.participant_id != *raw_p_id
                                    {
                                        return Err(ForgeError::Internal(format!(
                                            "FATAL RESTORE RECOVERY FAILURE: Staged storage node ack identity '{}' != expected '{}'",
                                            ack.participant_id, expected_node_id
                                        )));
                                    }
                                    if !in_maint {
                                        client.enter_maintenance(expected_gen).await.map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed to enter maintenance on storage node '{}' before commit: {}",
                                                expected_node_id, e
                                            ))
                                        })?;
                                    }
                                    client
                                        .commit_restore(&snapshot_id, expected_gen)
                                        .await
                                        .map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed to commit staged restore '{}' on storage node '{}': {}",
                                                snapshot_id, expected_node_id, e
                                            ))
                                        })?;

                                    let (post_maint, post_gen, _, post_id) =
                                        client.query_node_details().await.map_err(|e| {
                                            ForgeError::Internal(format!(
                                                "FATAL RESTORE RECOVERY FAILURE: Failed post-commit query on storage node '{}': {}",
                                                expected_node_id, e
                                            ))
                                        })?;
                                    if post_id.as_deref() != Some(expected_node_id)
                                        || post_maint
                                        || post_gen != expected_gen
                                    {
                                        return Err(ForgeError::Internal(format!(
                                            "FATAL RESTORE RECOVERY FAILURE: Storage node '{}' post-commit state invalid (id={:?}, maint={}, gen={}, expected={})",
                                            expected_node_id, post_id, post_maint, post_gen, expected_gen
                                        )));
                                    }
                                }
                                Ok(None) => {
                                    return Err(ForgeError::Internal(format!(
                                        "FATAL RESTORE RECOVERY FAILURE: Storage node '{}' generation ({}) != target ({}) (in_maint={}) and no staged restore '{}'",
                                        expected_node_id, node_gen, expected_gen, in_maint, snapshot_id
                                    )));
                                }
                                Err(e) => {
                                    return Err(ForgeError::Internal(format!(
                                        "FATAL RESTORE RECOVERY FAILURE: Failed querying staged restore '{}' on storage node '{}': {}",
                                        snapshot_id, expected_node_id, e
                                    )));
                                }
                            }
                        }

                        journal.record_participant_recovery(
                            raw_p_id,
                            true,
                            prev_progress.finalized,
                            Some(&target_gen),
                        )?;

                        if !prev_progress.finalized {
                            client.finalize_restore(&snapshot_id).await.map_err(|e| {
                                ForgeError::Internal(format!(
                                    "FATAL RESTORE RECOVERY FAILURE: Storage node '{}' finalize_restore('{}') failed: {}",
                                    expected_node_id, snapshot_id, e
                                ))
                            })?;
                            journal.record_participant_recovery(
                                raw_p_id,
                                true,
                                true,
                                Some(&target_gen),
                            )?;
                        }
                    }
                    other => {
                        return Err(ForgeError::Internal(format!(
                            "FATAL RESTORE RECOVERY FAILURE: Unsupported participant role '{}'",
                            other
                        )));
                    }
                }
            }

            // Finalize local storage old generation only after all required participants have converged
            let old_dir = storage_root.join(format!(".old_generation_{}", journal.restore_id));
            if old_dir.exists() {
                std::fs::remove_dir_all(&old_dir).map_err(|e| {
                    ForgeError::Internal(format!(
                        "FATAL RESTORE RECOVERY FAILURE: Failed to finalize old generation directory {}: {}",
                        old_dir.display(),
                        e
                    ))
                })?;
            }
            if required_participants.iter().any(|p| p == "spine_storage") {
                journal.record_participant_recovery(
                    "spine_storage",
                    true,
                    true,
                    Some(&target_gen),
                )?;
            }

            // Verify every single required participant is both converged and finalized before completing
            for req_id in &required_participants {
                let state = journal.participant_progress.get(req_id).ok_or_else(|| {
                    ForgeError::Internal(format!(
                        "FATAL RESTORE RECOVERY FAILURE: Required participant '{}' missing durable convergence state",
                        req_id
                    ))
                })?;
                if !state.converged
                    || !state.finalized
                    || state.verified_generation.as_deref() != Some(&target_gen)
                {
                    return Err(ForgeError::Internal(format!(
                        "FATAL RESTORE RECOVERY FAILURE: Required participant '{}' did not finish convergence+finalization (converged={}, finalized={}, gen={:?})",
                        req_id, state.converged, state.finalized, state.verified_generation
                    )));
                }
            }

            journal.persist(RestorePhase::Completed)?;
            Self::remove(storage_root)?;
            tracing::info!(
                "Global restore '{}' successfully converged and finalized across all {} required participants. Journal removed.",
                journal.restore_id,
                required_participants.len()
            );

            return Ok(Some(format!(
                "Global restore '{}' fully recovered, committed, and rolled forward across all {} required participants to target generation '{}'.",
                journal.restore_id,
                required_participants.len(),
                target_gen
            )));
        }

        if let Some(pool) = pool {
            // Check if database is empty or initialized
            let table_count: (i64,) = sqlx::query_as(
                "SELECT count(*)::bigint FROM information_schema.tables WHERE table_schema = 'public' AND table_name NOT LIKE 'pg_%' AND table_name NOT LIKE '_spine_%'",
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
                            Some(ref s_gen) if s_gen == &db_gen => {
                                tracing::info!(
                                    "Verified consistent generation token '{}' across database and storage root.",
                                    db_gen
                                );
                                reconciled_storage_gen = Some(db_gen);
                            }
                            Some(ref s_gen) => {
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

        // Parse numeric cluster fencing generation ONCE before RPCs if any cluster nodes configured
        let expected_cluster_fencing_gen = if coordinator.is_some() || !storage_nodes.is_empty() {
            match reconciled_storage_gen {
                Some(ref s_gen) => Some(Self::parse_generation_u64(s_gen)?),
                None => None,
            }
        } else {
            None
        };

        // Reconcile and verify coordinator generation if configured
        if let Some(coord) = coordinator {
            let (in_maint, coord_gen, _, responding_id) =
                coord.query_node_details().await.map_err(|e| {
                    ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Failed to query coordinator details during recovery: {}",
                        e
                    ))
                })?;

            if in_maint {
                return Err(ForgeError::Internal(
                    "FATAL STARTUP BLOCKED: Coordinator is in maintenance mode on startup."
                        .to_string(),
                ));
            }
            if responding_id.as_deref() != Some("coordinator") {
                return Err(ForgeError::Internal(format!(
                    "FATAL STARTUP BLOCKED: Coordinator responded with identity {:?}, expected 'coordinator'",
                    responding_id
                )));
            }

            if let Some(expected_fencing_gen) = expected_cluster_fencing_gen {
                if coord_gen != expected_fencing_gen {
                    return Err(ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Coordinator generation ({}) does not match cluster storage generation ({})",
                        coord_gen, expected_fencing_gen
                    )));
                }
            }
        }

        // Reconcile and verify storage nodes if configured
        for (expected_id, addr) in storage_nodes {
            let client = crate::ClusterNodeClient::new(addr.clone());
            let (in_maint, node_gen, _, responding_id) =
                client.query_node_details().await.map_err(|e| {
                    ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Failed to query storage node '{}' at {}: {}",
                        expected_id, addr, e
                    ))
                })?;

            if in_maint {
                return Err(ForgeError::Internal(format!(
                    "FATAL STARTUP BLOCKED: Storage node '{}' at {} is in maintenance mode on startup.",
                    expected_id, addr
                )));
            }

            match responding_id {
                Some(ref r_id) if r_id == expected_id => {}
                other => {
                    return Err(ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Storage node at {} responded with ID {:?}, expected '{}'",
                        addr, other, expected_id
                    )));
                }
            }

            if let Some(expected_fencing_gen) = expected_cluster_fencing_gen {
                if node_gen != expected_fencing_gen {
                    return Err(ForgeError::Internal(format!(
                        "FATAL STARTUP BLOCKED: Storage node '{}' generation ({}) does not match cluster storage generation ({})",
                        expected_id, node_gen, expected_fencing_gen
                    )));
                }
            }
        }

        Ok(sync_msg)
    }

    /// Asynchronous crash detection, cleanup and cross-resource generation verification.
    /// Checks both the durable journal file AND database generation identity.
    /// FAILS CLOSED if incomplete restoration or generation mismatch is detected.
    pub async fn handle_startup_recovery(
        storage_root: &Path,
        pool: Option<&sqlx::PgPool>,
    ) -> Result<Option<String>, ForgeError> {
        Self::handle_startup_recovery_cluster(storage_root, pool, None, &[]).await
    }

    /// Asynchronous crash detection, cleanup and full cluster generation verification across all 7 participants.
    /// Checks durable journal, database generation identity, coordinator generation, and storage nodes.
    /// FAILS CLOSED if incomplete restoration or generation mismatch is detected.
    pub async fn handle_startup_recovery_cluster(
        storage_root: &Path,
        pool: Option<&sqlx::PgPool>,
        coordinator: Option<&crate::CoordinatorClient>,
        storage_nodes: &[(String, String)],
    ) -> Result<Option<String>, ForgeError> {
        type DbCheckFn = fn(&str) -> Result<Option<String>, ForgeError>;
        let no_check: Option<DbCheckFn> = None;
        Self::handle_startup_recovery_cluster_with_db_check(
            storage_root,
            no_check,
            pool,
            coordinator,
            storage_nodes,
        )
        .await
    }
}
