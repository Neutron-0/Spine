//! Cooperative repository maintenance locking (`.spine/LOCK`).

use std::fs::{self, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LockError {
    #[error("Repository maintenance lock held by active process: {0}")]
    AlreadyLocked(String),
    #[error("I/O error acquiring lock: {0}")]
    Io(#[from] io::Error),
}

/// Cooperative file lock protecting maintenance operations.
pub struct RepoLock {
    path: PathBuf,
}

#[cfg(windows)]
fn is_pid_alive(pid: u32) -> bool {
    #[link(name = "kernel32")]
    extern "system" {
        fn OpenProcess(desired_access: u32, inherit_handle: i32, process_id: u32) -> isize;
        fn CloseHandle(handle: isize) -> i32;
        fn GetExitCodeProcess(handle: isize, exit_code: *mut u32) -> i32;
    }
    const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
    const STILL_ACTIVE: u32 = 259;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if handle == 0 {
            return false;
        }
        let mut exit_code: u32 = 0;
        let res = GetExitCodeProcess(handle, &mut exit_code);
        CloseHandle(handle);
        res != 0 && exit_code == STILL_ACTIVE
    }
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("kill")
        .args(["-0", &pid.to_string()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[cfg(not(any(windows, unix)))]
fn is_pid_alive(_pid: u32) -> bool {
    true
}

fn parse_lock_pid(content: &str) -> Option<u32> {
    for part in content.split(',') {
        let part = part.trim();
        if let Some(pid_str) = part.strip_prefix("pid:") {
            if let Ok(pid) = pid_str.trim().parse::<u32>() {
                return Some(pid);
            }
        }
    }
    None
}

impl RepoLock {
    /// Attempts to acquire the maintenance lock at `.spine/LOCK`.
    /// Recovers automatically if the lock is held by a dead process (stale lock).
    pub fn acquire(spine_dir: impl AsRef<Path>, operation: &str) -> Result<Self, LockError> {
        let path = spine_dir.as_ref().join("LOCK");

        for attempt in 0..2 {
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(mut file) => {
                    use std::io::Write;
                    let pid = std::process::id();
                    let _ = writeln!(file, "pid: {}, operation: {}", pid, operation);
                    return Ok(Self { path });
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {
                    let holder =
                        fs::read_to_string(&path).unwrap_or_else(|_| "unknown".to_string());
                    if attempt == 0 {
                        if let Some(pid) = parse_lock_pid(&holder) {
                            if !is_pid_alive(pid) {
                                // Previous process crashed or exited without removing lock
                                let _ = fs::remove_file(&path);
                                continue;
                            }
                        }
                    }
                    return Err(LockError::AlreadyLocked(holder.trim().to_string()));
                }
                Err(e) => return Err(LockError::Io(e)),
            }
        }

        let holder = fs::read_to_string(&path).unwrap_or_else(|_| "unknown".to_string());
        Err(LockError::AlreadyLocked(holder.trim().to_string()))
    }
}

impl Drop for RepoLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}
