use crate::MergeError;
use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use std::fs;
use std::path::Path;

/// Active pending rebase state persisted on disk at `.spine/rebase-apply/state.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RebaseState {
    pub onto: ObjectId,
    pub orig_head: ObjectId,
    pub branch_name: Option<String>,
    pub remaining_commits: Vec<ObjectId>,
    pub current_commit: Option<ObjectId>,
    pub old_to_new: Vec<(ObjectId, ObjectId)>,
}

impl RebaseState {
    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> Result<(), MergeError> {
        let dir = spine_dir.as_ref().join("rebase-apply");
        fs::create_dir_all(&dir)?;
        let content = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(dir.join("state.json"), content)?;
        Ok(())
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> Result<Option<Self>, MergeError> {
        let file = spine_dir.as_ref().join("rebase-apply").join("state.json");
        if !file.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file)?;
        let state: Self = serde_json::from_str(&content)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Some(state))
    }

    pub fn clear<P: AsRef<Path>>(spine_dir: P) -> Result<(), MergeError> {
        let dir = spine_dir.as_ref().join("rebase-apply");
        if dir.exists() {
            fs::remove_dir_all(dir)?;
        }
        Ok(())
    }
}

/// Active pending cherry-pick state persisted on disk at `.spine/CHERRY_PICK_STATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CherryPickState {
    pub target_commit: ObjectId,
    pub orig_head: ObjectId,
    pub mainline: Option<usize>,
}

impl CherryPickState {
    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> Result<(), MergeError> {
        let file = spine_dir.as_ref().join("CHERRY_PICK_STATE");
        let content = toml::to_string_pretty(self)?;
        fs::write(file, content)?;
        Ok(())
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> Result<Option<Self>, MergeError> {
        let file = spine_dir.as_ref().join("CHERRY_PICK_STATE");
        if !file.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file)?;
        let state: Self = toml::from_str(&content)?;
        Ok(Some(state))
    }

    pub fn clear<P: AsRef<Path>>(spine_dir: P) -> Result<(), MergeError> {
        let file = spine_dir.as_ref().join("CHERRY_PICK_STATE");
        if file.exists() {
            fs::remove_file(file)?;
        }
        Ok(())
    }
}

/// Active pending revert state persisted on disk at `.spine/REVERT_STATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RevertState {
    pub target_commit: ObjectId,
    pub orig_head: ObjectId,
    pub mainline: Option<usize>,
}

impl RevertState {
    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> Result<(), MergeError> {
        let file = spine_dir.as_ref().join("REVERT_STATE");
        let content = toml::to_string_pretty(self)?;
        fs::write(file, content)?;
        Ok(())
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> Result<Option<Self>, MergeError> {
        let file = spine_dir.as_ref().join("REVERT_STATE");
        if !file.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(file)?;
        let state: Self = toml::from_str(&content)?;
        Ok(Some(state))
    }

    pub fn clear<P: AsRef<Path>>(spine_dir: P) -> Result<(), MergeError> {
        let file = spine_dir.as_ref().join("REVERT_STATE");
        if file.exists() {
            fs::remove_file(file)?;
        }
        Ok(())
    }
}
