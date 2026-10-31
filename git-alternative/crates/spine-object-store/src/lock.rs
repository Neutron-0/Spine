//! Cooperative repository maintenance locking (`.spine/LOCK`).

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("Repository maintenance lock held by another process: {0}")]
    AlreadyLocked(String),
    #[error("I/O error acquiring lock: {0}")]
    Io(#[from] io::Error),
}

/// Cooperative file lock protecting maintenance operations.
pub struct RepoLock {
    path: PathBuf,
}

impl RepoLock {
    /// Attempts to acquire the maintenance lock at `.spine/LOCK`.
    pub fn acquire(spine_dir: impl AsRef<Path>, operation: &str) -> Result<Self, LockError> {
        let path = spine_dir.as_ref().join("LOCK");

        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                use std::io::Write;
                let pid = std::process::id();
                let _ = writeln!(file, "pid: {}, operation: {}", pid, operation);
                Ok(Self { path })
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                let holder = fs::read_to_string(&path).unwrap_or_else(|_| "unknown".to_string());
                Err(LockError::AlreadyLocked(holder.trim().to_string()))
            }
            Err(e) => Err(LockError::Io(e)),
        }
    }
}

impl Drop for RepoLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
