use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BisectState {
    pub bad_commit: ObjectId,
    pub good_commits: Vec<ObjectId>,
    pub skipped_commits: Vec<ObjectId>,
    pub orig_head: ObjectId,
    pub orig_branch: Option<String>,
    pub current_commit: Option<ObjectId>,
}

impl BisectState {
    pub fn new(
        bad_commit: ObjectId,
        good_commits: Vec<ObjectId>,
        orig_head: ObjectId,
        orig_branch: Option<String>,
    ) -> Self {
        Self {
            bad_commit,
            good_commits,
            skipped_commits: Vec::new(),
            orig_head,
            orig_branch,
            current_commit: None,
        }
    }

    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> io::Result<()> {
        let p = spine_dir.as_ref().join("BISECT_STATE");
        let content = toml::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(p, content)?;
        Ok(())
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> io::Result<Option<Self>> {
        let p = spine_dir.as_ref().join("BISECT_STATE");
        if !p.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(p)?;
        let state: Self = toml::from_str(&content)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(Some(state))
    }

    pub fn clear<P: AsRef<Path>>(spine_dir: P) -> io::Result<()> {
        let p = spine_dir.as_ref().join("BISECT_STATE");
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok(())
    }
}
