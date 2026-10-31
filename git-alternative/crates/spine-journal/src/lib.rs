use chrono::Utc;
use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum JournalError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Journal transaction not found: {0}")]
    TransactionNotFound(String),
    #[error("Invalid transaction state: {0}")]
    InvalidState(String),
    #[error("Nothing to undo")]
    NothingToUndo,
    #[error("Nothing to redo")]
    NothingToRedo,
    #[error("Corrupted journal entry: {0}")]
    CorruptedEntry(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    Commit,
    BranchCreate,
    BranchDelete,
    BranchRename,
    Switch,
    Merge,
    MergeAbort,
    Undo,
    Redo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransactionStatus {
    Prepared,
    Applying,
    Committed,
    Aborted,
    RecoveryRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepoStateSnapshot {
    pub head_ref: String,
    pub branch_name: Option<String>,
    pub branch_tip: Option<ObjectId>,
    pub index_checksum: String,
    pub tree_id: Option<ObjectId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionRecord {
    pub sequence: u64,
    pub id: String,
    pub timestamp_nanos: i64,
    pub kind: OperationKind,
    pub description: String,
    pub status: TransactionStatus,
    pub before_state: RepoStateSnapshot,
    pub after_state: Option<RepoStateSnapshot>,
    pub target_tx_id: Option<String>,
    pub abort_reason: Option<String>,
}

pub struct Journal {
    base_dir: PathBuf,
    records_dir: PathBuf,
}

impl Journal {
    pub fn new(journal_dir: impl AsRef<Path>) -> Result<Self, JournalError> {
        let base_dir = journal_dir.as_ref().to_path_buf();
        let records_dir = base_dir.join("records");

        fs::create_dir_all(&records_dir)?;

        Ok(Self {
            base_dir,
            records_dir,
        })
    }

    fn head_file(&self) -> PathBuf {
        self.base_dir.join("HEAD")
    }

    fn record_path(&self, seq: u64, id: &str) -> PathBuf {
        self.records_dir.join(format!("{:08}-{}.json", seq, id))
    }

    pub fn next_sequence(&self) -> Result<u64, JournalError> {
        let records = self.list_records()?;
        Ok(records.last().map(|r| r.sequence + 1).unwrap_or(1))
    }

    pub fn prepare_transaction(
        &mut self,
        kind: OperationKind,
        description: String,
        before: RepoStateSnapshot,
        target_tx_id: Option<String>,
    ) -> Result<TransactionRecord, JournalError> {
        let seq = self.next_sequence()?;
        let id = Uuid::new_v4().to_string();
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let record = TransactionRecord {
            sequence: seq,
            id: id.clone(),
            timestamp_nanos: now,
            kind,
            description,
            status: TransactionStatus::Prepared,
            before_state: before,
            after_state: None,
            target_tx_id,
            abort_reason: None,
        };

        self.write_record_atomic(&record)?;
        Ok(record)
    }

    pub fn mark_applying(&mut self, tx_id: &str) -> Result<(), JournalError> {
        let mut record = self.find_record_by_id(tx_id)?;
        if record.status != TransactionStatus::Prepared {
            return Err(JournalError::InvalidState(format!(
                "Cannot transition transaction {} to Applying from status {:?}",
                tx_id, record.status
            )));
        }
        record.status = TransactionStatus::Applying;
        self.write_record_atomic(&record)?;
        Ok(())
    }

    pub fn commit_transaction(
        &mut self,
        tx_id: &str,
        after: RepoStateSnapshot,
    ) -> Result<(), JournalError> {
        let mut record = self.find_record_by_id(tx_id)?;
        if record.status != TransactionStatus::Prepared
            && record.status != TransactionStatus::Applying
        {
            return Err(JournalError::InvalidState(format!(
                "Cannot commit transaction {} in status {:?}",
                tx_id, record.status
            )));
        }

        record.status = TransactionStatus::Committed;
        record.after_state = Some(after);

        self.write_record_atomic(&record)?;
        self.set_head(&record.id)?;
        Ok(())
    }

    pub fn abort_transaction(&mut self, tx_id: &str, reason: &str) -> Result<(), JournalError> {
        let mut record = self.find_record_by_id(tx_id)?;
        record.status = TransactionStatus::Aborted;
        record.abort_reason = Some(reason.to_string());
        self.write_record_atomic(&record)?;
        Ok(())
    }

    pub fn mark_recovery_required(
        &mut self,
        tx_id: &str,
        reason: &str,
    ) -> Result<(), JournalError> {
        let mut record = self.find_record_by_id(tx_id)?;
        record.status = TransactionStatus::RecoveryRequired;
        record.abort_reason = Some(reason.to_string());
        self.write_record_atomic(&record)?;
        Ok(())
    }

    pub fn recover_with<F>(
        &mut self,
        mut rollback_fn: F,
    ) -> Result<Vec<TransactionRecord>, JournalError>
    where
        F: FnMut(&TransactionRecord) -> Result<(), String>,
    {
        let records = self.list_records()?;
        let mut recovered = Vec::new();
        for mut r in records.into_iter().rev() {
            if r.status == TransactionStatus::Prepared || r.status == TransactionStatus::Applying {
                match rollback_fn(&r) {
                    Ok(()) => {
                        r.status = TransactionStatus::Aborted;
                        r.abort_reason = Some(format!(
                            "Interrupted by crash or abnormal termination; rolled back ({:?})",
                            r.kind
                        ));
                    }
                    Err(err) => {
                        r.status = TransactionStatus::RecoveryRequired;
                        r.abort_reason = Some(format!("Rollback failed: {}", err));
                    }
                }
                self.write_record_atomic(&r)?;
                recovered.push(r);
            }
        }
        Ok(recovered)
    }

    pub fn check_and_recover(&mut self) -> Result<Option<TransactionRecord>, JournalError> {
        let recovered = self.recover_with(|_| Ok(()))?;
        Ok(recovered.into_iter().next())
    }

    pub fn set_head(&self, tx_id: &str) -> Result<(), JournalError> {
        spine_core::atomic_write_file(&self.head_file(), tx_id.as_bytes())?;
        Ok(())
    }

    pub fn get_head_id(&self) -> Result<Option<String>, JournalError> {
        let h = self.head_file();
        if !h.exists() {
            return Ok(None);
        }
        let id = fs::read_to_string(h)?.trim().to_string();
        if id.is_empty() {
            Ok(None)
        } else {
            Ok(Some(id))
        }
    }

    pub fn get_head(&self) -> Result<Option<TransactionRecord>, JournalError> {
        if let Some(id) = self.get_head_id()? {
            self.find_record_by_id(&id).map(Some)
        } else {
            Ok(None)
        }
    }

    pub fn list_records(&self) -> Result<Vec<TransactionRecord>, JournalError> {
        if !self.records_dir.exists() {
            return Ok(Vec::new());
        }

        let mut paths = Vec::new();
        for entry in fs::read_dir(&self.records_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("json") {
                paths.push(path);
            }
        }

        paths.sort();

        let mut records = Vec::new();
        for path in paths {
            let bytes = fs::read(&path)?;
            let record: TransactionRecord = serde_json::from_slice(&bytes)
                .map_err(|e| JournalError::CorruptedEntry(format!("{}: {}", path.display(), e)))?;
            records.push(record);
        }

        Ok(records)
    }

    pub fn find_record_by_id(&self, tx_id: &str) -> Result<TransactionRecord, JournalError> {
        for r in self.list_records()? {
            if r.id == tx_id {
                return Ok(r);
            }
        }
        Err(JournalError::TransactionNotFound(tx_id.to_string()))
    }

    fn write_record_atomic(&self, record: &TransactionRecord) -> Result<(), JournalError> {
        let json_bytes = serde_json::to_vec_pretty(record)?;
        let final_path = self.record_path(record.sequence, &record.id);
        spine_core::atomic_write_file(&final_path, &json_bytes)?;
        Ok(())
    }

    /// Step back in journal to undo the last committed operation
    pub fn plan_undo(&self) -> Result<TransactionRecord, JournalError> {
        let head_id = self.get_head_id()?.ok_or(JournalError::NothingToUndo)?;
        let head_record = self.find_record_by_id(&head_id)?;

        if head_record.status != TransactionStatus::Committed {
            return Err(JournalError::InvalidState(format!(
                "Cannot undo uncommitted transaction {}",
                head_id
            )));
        }

        // We can undo standard mutating operations
        Ok(head_record)
    }

    /// Step forward in journal to redo an undone operation
    pub fn plan_redo(&self) -> Result<TransactionRecord, JournalError> {
        let head_id = self.get_head_id()?.ok_or(JournalError::NothingToRedo)?;
        let head_record = self.find_record_by_id(&head_id)?;

        if head_record.kind != OperationKind::Undo {
            return Err(JournalError::NothingToRedo);
        }

        let target_id = head_record.target_tx_id.as_ref().ok_or_else(|| {
            JournalError::InvalidState("Undo record missing target transaction ID".to_string())
        })?;

        let target_record = self.find_record_by_id(target_id)?;
        Ok(target_record)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn dummy_snapshot(name: &str) -> RepoStateSnapshot {
        RepoStateSnapshot {
            head_ref: format!("refs/heads/{}", name),
            branch_name: Some(name.to_string()),
            branch_tip: Some(ObjectId::from_bytes([0x01; 32])),
            index_checksum: "dummy_checksum".to_string(),
            tree_id: Some(ObjectId::from_bytes([0x02; 32])),
        }
    }

    #[test]
    fn test_journal_prepare_and_commit() {
        let dir = tempdir().unwrap();
        let mut journal = Journal::new(dir.path()).unwrap();

        let before = dummy_snapshot("main");
        let rec = journal
            .prepare_transaction(
                OperationKind::Commit,
                "Initial commit".to_string(),
                before.clone(),
                None,
            )
            .unwrap();

        assert_eq!(rec.sequence, 1);
        assert_eq!(rec.status, TransactionStatus::Prepared);

        let mut after = before.clone();
        after.branch_tip = Some(ObjectId::from_bytes([0x03; 32]));

        journal.commit_transaction(&rec.id, after.clone()).unwrap();

        let head = journal.get_head().unwrap().unwrap();
        assert_eq!(head.id, rec.id);
        assert_eq!(head.status, TransactionStatus::Committed);
        assert_eq!(head.after_state, Some(after));
    }

    #[test]
    fn test_journal_crash_recovery() {
        let dir = tempdir().unwrap();
        let mut journal = Journal::new(dir.path()).unwrap();

        let before = dummy_snapshot("main");
        let rec = journal
            .prepare_transaction(
                OperationKind::Switch,
                "Switch main -> feature".to_string(),
                before,
                None,
            )
            .unwrap();

        // Simulate crash: transaction left in Prepared status
        let recovered = journal.check_and_recover().unwrap();
        assert!(recovered.is_some());
        let recov = recovered.unwrap();
        assert_eq!(recov.id, rec.id);
        assert_eq!(recov.status, TransactionStatus::Aborted);
        assert!(recov.abort_reason.unwrap().contains("crash"));

        // Second check should find nothing
        let second = journal.check_and_recover().unwrap();
        assert!(second.is_none());
    }

    #[test]
    fn test_journal_undo_redo_planning() {
        let dir = tempdir().unwrap();
        let mut journal = Journal::new(dir.path()).unwrap();

        let s1 = dummy_snapshot("main");
        let rec1 = journal
            .prepare_transaction(
                OperationKind::Commit,
                "Commit 1".to_string(),
                s1.clone(),
                None,
            )
            .unwrap();
        let s2 = dummy_snapshot("feature");
        journal.commit_transaction(&rec1.id, s2.clone()).unwrap();

        // Plan undo
        let to_undo = journal.plan_undo().unwrap();
        assert_eq!(to_undo.id, rec1.id);

        // Record the undo operation
        let undo_rec = journal
            .prepare_transaction(
                OperationKind::Undo,
                format!("Undo {}", rec1.id),
                s2.clone(),
                Some(rec1.id.clone()),
            )
            .unwrap();
        journal
            .commit_transaction(&undo_rec.id, s1.clone())
            .unwrap();

        // Now plan redo
        let to_redo = journal.plan_redo().unwrap();
        assert_eq!(to_redo.id, rec1.id);
    }

    #[test]
    fn test_journal_state_machine_applying_and_recovery_required() {
        let dir = tempdir().unwrap();
        let mut journal = Journal::new(dir.path()).unwrap();

        let s1 = dummy_snapshot("main");
        let rec = journal
            .prepare_transaction(
                OperationKind::Commit,
                "Multi-step pack operation".to_string(),
                s1.clone(),
                None,
            )
            .unwrap();

        // Transition Prepared -> Applying
        journal.mark_applying(&rec.id).unwrap();
        let loaded = journal.find_record_by_id(&rec.id).unwrap();
        assert_eq!(loaded.status, TransactionStatus::Applying);

        // Simulate failed rollback
        let recovered = journal
            .recover_with(|_tx| Err("I/O lock failure during rollback".to_string()))
            .unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, TransactionStatus::RecoveryRequired);
        assert!(recovered[0]
            .abort_reason
            .as_ref()
            .unwrap()
            .contains("I/O lock failure"));

        // Idempotency: second recovery call does not find Applying/Prepared transactions
        let second = journal.recover_with(|_tx| Ok(())).unwrap();
        assert!(second.is_empty());
    }
}
