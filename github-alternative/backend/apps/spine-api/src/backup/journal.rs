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
    /// Database snapshot verified and restored to target database before storage publication.
    DatabaseRestored,
    /// Staged storage atomically published to live repository storage root.
    StoragePublished,
    /// Entire restore sequence completed successfully.
    Completed,
    /// Restore failed with specific diagnostic error.
    Failed(String),
}

/// Durable cross-resource restoration journal persisted as `.spine_restore_journal.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RestoreJournal {
    pub restore_id: String,
    pub phase: RestorePhase,
    pub input_archive: PathBuf,
    pub target_storage_root: PathBuf,
    pub staging_storage_root: PathBuf,
    pub db_snapshot_staged: Option<PathBuf>,
    pub started_at_nanos: i64,
    pub updated_at_nanos: i64,
}

impl RestoreJournal {
    pub const JOURNAL_FILE_NAME: &'static str = ".spine_restore_journal.json";

    /// Initialize a new restoration journal tracking the state machine.
    pub fn new(
        input_archive: &Path,
        target_storage_root: &Path,
        staging_storage_root: &Path,
    ) -> Self {
        let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        Self {
            restore_id: format!("rst_{}", uuid::Uuid::new_v4().simple()),
            phase: RestorePhase::Initialized,
            input_archive: input_archive.to_path_buf(),
            target_storage_root: target_storage_root.to_path_buf(),
            staging_storage_root: staging_storage_root.to_path_buf(),
            db_snapshot_staged: None,
            started_at_nanos: now,
            updated_at_nanos: now,
        }
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

    /// Startup crash detection and idempotent recovery.
    /// Cleans up uncommitted staging artifacts from interrupted restoration runs.
    pub fn handle_startup_recovery(storage_root: &Path) -> Result<Option<String>, ForgeError> {
        if let Some(journal) = Self::load(storage_root)? {
            match journal.phase {
                RestorePhase::Completed => {
                    let _ = Self::remove(storage_root);
                    Ok(None)
                }
                RestorePhase::Initialized
                | RestorePhase::StagedStorage
                | RestorePhase::Failed(_) => {
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
                RestorePhase::DatabaseRestored | RestorePhase::StoragePublished => {
                    tracing::error!(
                        "CRITICAL: Detected incomplete restore session '{}' in phase '{:?}'. Target database and storage cutover was in progress when interrupted. Manual operator verification required.",
                        journal.restore_id,
                        journal.phase
                    );
                    Ok(Some(format!(
                        "CRITICAL: Incomplete restore cutover detected ({:?}).",
                        journal.phase
                    )))
                }
            }
        } else {
            Ok(None)
        }
    }
}
