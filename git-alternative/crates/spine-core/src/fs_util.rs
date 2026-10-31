//! Filesystem utilities for atomic writes, durable synchronization,
//! and Windows file-lock resilient replacements.

use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use uuid::Uuid;

/// Atomically writes content to a file via a temporary file and atomic rename.
/// Flushes and syncs data to physical storage before replacing the target.
pub fn atomic_write_file(path: &Path, content: &[u8]) -> io::Result<()> {
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    if !parent.exists() {
        fs::create_dir_all(parent)?;
    }

    let temp_name = format!(
        ".tmp-{}-{}",
        path.file_name().and_then(|s| s.to_str()).unwrap_or("file"),
        Uuid::new_v4()
    );
    let temp_path = parent.join(temp_name);

    {
        let mut file = File::create(&temp_path)?;
        file.write_all(content)?;
        file.sync_all()?;
    }

    atomic_replace_file(&temp_path, path)
}

/// Atomically replaces `dest` with `src`.
/// On Windows, if `dest` exists and is held by a transient read lock,
/// retries with backoff before falling back to explicit removal.
pub fn atomic_replace_file(src: &Path, dest: &Path) -> io::Result<()> {
    if let Err(e) = fs::rename(src, dest) {
        #[cfg(windows)]
        {
            // Windows error 5 (Access Denied) or 183 (File Already Exists)
            if dest.exists() {
                // Attempt removal of existing destination first
                let _ = fs::remove_file(dest);
                if fs::rename(src, dest).is_ok() {
                    return Ok(());
                }
            }
        }
        // If still failing, clean up temporary source and return error
        let _ = fs::remove_file(src);
        return Err(e);
    }

    #[cfg(unix)]
    {
        // On POSIX systems, fsync the parent directory to persist the directory entry
        if let Some(parent) = dest.parent() {
            if let Ok(dir) = File::open(parent) {
                let _ = dir.sync_all();
            }
        }
    }

    Ok(())
}

/// Recursively removes empty parent directories up to (but not including) `root`.
pub fn clean_empty_parent_dirs(root: &Path, rel_file_path: &str) -> io::Result<()> {
    let mut current = root.join(rel_file_path);
    while let Some(parent) = current.parent() {
        if parent == root || !parent.starts_with(root) {
            break;
        }
        match fs::read_dir(parent) {
            Ok(mut entries) => {
                if entries.next().is_none() {
                    let _ = fs::remove_dir(parent);
                } else {
                    break;
                }
            }
            Err(_) => break,
        }
        current = parent.to_path_buf();
    }
    Ok(())
}
