//! # spine-worktree
//!
//! Working tree traversal, ignore rules evaluation, stat-cache diffing,
//! and staging mechanics for Spine repositories.

use spine_core::{FileMode, ObjectId, ObjectType, TreeEntry};
use spine_format::deserialize_tree;
use spine_index::{Index, IndexEntry, StageStatus};
use spine_object_store::{ObjectStore, ObjectStoreError};
use std::collections::BTreeMap;
use std::fs::{self, Metadata};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorktreeError {
    #[error("Unsafe path traversal detected: {0}")]
    UnsafePath(String),
    #[error("File not found in worktree: {0}")]
    FileNotFound(String),
    #[error("Cannot add directory itself without files: {0}")]
    EmptyDirectory(String),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
    #[error("Index error: {0}")]
    Index(#[from] spine_index::IndexError),
    #[error("Format error: {0}")]
    Format(#[from] spine_format::FormatError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

/// Normalizes a path to a safe, relative UTF-8 string with forward slashes (`/`).
pub fn normalize_rel_path<P: AsRef<Path>>(path: P) -> Result<String, WorktreeError> {
    let p = path.as_ref();
    let mut parts = Vec::new();

    for comp in p.components() {
        match comp {
            Component::Normal(os_str) => {
                let s = os_str
                    .to_str()
                    .ok_or_else(|| WorktreeError::UnsafePath("Non-UTF8 path component".into()))?;
                if s.contains(':') || s.contains('\\') {
                    return Err(WorktreeError::UnsafePath(format!(
                        "Invalid characters in: {}",
                        s
                    )));
                }
                parts.push(s);
            }
            Component::CurDir => continue,
            Component::ParentDir => {
                return Err(WorktreeError::UnsafePath(
                    "Path traversal (..) not permitted".into(),
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(WorktreeError::UnsafePath(
                    "Absolute paths not permitted".into(),
                ));
            }
        }
    }

    if parts.is_empty() {
        return Ok(String::new());
    }

    Ok(parts.join("/"))
}

/// Simple pattern matcher for `.spineignore`
#[derive(Debug, Default, Clone)]
pub struct IgnoreRules {
    patterns: Vec<String>,
}

impl IgnoreRules {
    pub fn load_from_dir<P: AsRef<Path>>(root: P) -> Self {
        let mut rules = Self::default();
        // Mandatory internal ignores
        rules.patterns.push(".spine".into());
        rules.patterns.push(".spine/".into());
        rules.patterns.push(".git".into());
        rules.patterns.push(".git/".into());

        let ignore_file = root.as_ref().join(".spineignore");
        if ignore_file.exists() {
            if let Ok(content) = fs::read_to_string(ignore_file) {
                for line in content.lines() {
                    let trimmed = line.trim();
                    if !trimmed.is_empty() && !trimmed.starts_with('#') {
                        rules.patterns.push(trimmed.replace('\\', "/"));
                    }
                }
            }
        }

        rules
    }

    pub fn is_ignored(&self, rel_path: &str) -> bool {
        for pattern in &self.patterns {
            if pattern.ends_with('/') {
                let prefix = pattern.trim_end_matches('/');
                if rel_path == prefix || rel_path.starts_with(&format!("{}/", prefix)) {
                    return true;
                }
            } else if let Some(suffix) = pattern.strip_prefix('*') {
                if rel_path.ends_with(suffix) {
                    return true;
                }
            } else if rel_path == pattern || rel_path.starts_with(&format!("{}/", pattern)) {
                return true;
            }
        }
        false
    }
}

/// Worktree status summary.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct StatusReport {
    pub staged_new: Vec<String>,
    pub staged_modified: Vec<String>,
    pub staged_deleted: Vec<String>,
    pub unstaged_modified: Vec<String>,
    pub unstaged_deleted: Vec<String>,
    pub untracked: Vec<String>,
}

impl StatusReport {
    pub fn is_clean(&self) -> bool {
        self.staged_new.is_empty()
            && self.staged_modified.is_empty()
            && self.staged_deleted.is_empty()
            && self.unstaged_modified.is_empty()
            && self.unstaged_deleted.is_empty()
            && self.untracked.is_empty()
    }
}

/// Extracts file mode, size, and timestamp from filesystem metadata.
pub fn metadata_to_stat(meta: &Metadata) -> (FileMode, u64, i64, u32) {
    let size = meta.len();
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt;
        let perms = meta.permissions().mode();
        if perms & 0o111 != 0 {
            FileMode::Executable
        } else {
            FileMode::Regular
        }
    };
    #[cfg(not(unix))]
    let mode = FileMode::Regular;

    let (mtime_secs, mtime_nanos) = match meta.modified() {
        Ok(time) => match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(dur) => (dur.as_secs() as i64, dur.subsec_nanos()),
            Err(e) => (
                -(e.duration().as_secs() as i64),
                e.duration().subsec_nanos(),
            ),
        },
        Err(_) => (0, 0),
    };

    (mode, size, mtime_secs, mtime_nanos)
}

/// Recursively scans working directory files, ignoring matching paths.
pub fn scan_worktree<P: AsRef<Path>>(
    root: P,
    ignore: &IgnoreRules,
) -> Result<BTreeMap<String, PathBuf>, WorktreeError> {
    let mut files = BTreeMap::new();
    let root_path = root.as_ref();
    scan_dir_recursive(root_path, root_path, ignore, &mut files)?;
    Ok(files)
}

fn scan_dir_recursive(
    base: &Path,
    current: &Path,
    ignore: &IgnoreRules,
    files: &mut BTreeMap<String, PathBuf>,
) -> Result<(), WorktreeError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        let rel = path
            .strip_prefix(base)
            .map_err(|_| WorktreeError::UnsafePath("Failed to strip prefix".into()))?;
        let norm = normalize_rel_path(rel)?;

        if norm.is_empty() || ignore.is_ignored(&norm) {
            continue;
        }

        let file_type = entry.file_type()?;
        if file_type.is_dir() {
            scan_dir_recursive(base, &path, ignore, files)?;
        } else if file_type.is_file() || file_type.is_symlink() {
            files.insert(norm, path);
        }
    }
    Ok(())
}

/// Flatten a committed tree structure recursively into a mapping of path -> ObjectId
pub fn flatten_tree<S: ObjectStore>(
    store: &S,
    tree_id: &ObjectId,
    prefix: &str,
    out: &mut BTreeMap<String, (FileMode, ObjectId)>,
) -> Result<(), WorktreeError> {
    let (_type, payload) = store.read_object(tree_id)?;
    let entries = deserialize_tree(&payload)?;

    for entry in entries {
        let full_path = if prefix.is_empty() {
            entry.name.clone()
        } else {
            format!("{}/{}", prefix, entry.name)
        };

        if entry.mode == FileMode::Directory {
            flatten_tree(store, &entry.object_id, &full_path, out)?;
        } else {
            out.insert(full_path, (entry.mode, entry.object_id));
        }
    }

    Ok(())
}

/// Stages a path into the index and writes the object blob into the object store.
pub fn stage_path<P: AsRef<Path>, S: ObjectStore>(
    root: P,
    target: P,
    index: &mut Index,
    store: &S,
    ignore: &IgnoreRules,
) -> Result<usize, WorktreeError> {
    let root_path = root.as_ref();
    let abs_target = if target.as_ref().is_absolute() {
        target.as_ref().to_path_buf()
    } else {
        root_path.join(target.as_ref())
    };

    if !abs_target.exists() {
        // Check if it was deleted from working copy but present in index
        let rel_target = abs_target
            .strip_prefix(root_path)
            .map_err(|_| WorktreeError::UnsafePath("Path not within repository root".into()))?;
        let norm_path = normalize_rel_path(rel_target)?;
        if index.get(&norm_path).is_some() {
            index.remove(&norm_path);
            return Ok(1);
        }
        return Err(WorktreeError::FileNotFound(norm_path));
    }

    let meta = fs::symlink_metadata(&abs_target)?;
    let mut staged_count = 0;

    if meta.is_dir() {
        let files = scan_worktree(&abs_target, ignore)?;
        for (_, full_file_path) in files {
            staged_count += stage_single_file(root_path, &full_file_path, index, store)?;
        }
    } else {
        staged_count += stage_single_file(root_path, &abs_target, index, store)?;
    }

    Ok(staged_count)
}

fn stage_single_file<S: ObjectStore>(
    root: &Path,
    file_path: &Path,
    index: &mut Index,
    store: &S,
) -> Result<usize, WorktreeError> {
    let rel = file_path
        .strip_prefix(root)
        .map_err(|_| WorktreeError::UnsafePath("File outside repo root".into()))?;
    let norm = normalize_rel_path(rel)?;

    let meta = fs::metadata(file_path)?;
    let (mode, size, mtime_secs, mtime_nanos) = metadata_to_stat(&meta);

    let mut contents = Vec::new();
    let mut f = fs::File::open(file_path)?;
    f.read_to_end(&mut contents)?;

    let object_id = store.write_object(ObjectType::Blob, &contents)?;

    index.add_or_update(IndexEntry {
        path: norm,
        mode,
        file_size: size,
        mtime_secs,
        mtime_nanos,
        inode: 0,
        object_id,
        stage_status: StageStatus::Staged,
    });

    Ok(1)
}

/// Computes the complete repository status by comparing working directory, index, and HEAD commit tree.
pub fn compute_status<P: AsRef<Path>, S: ObjectStore>(
    root: P,
    index: &Index,
    store: &S,
    head_tree_id: Option<&ObjectId>,
    ignore: &IgnoreRules,
) -> Result<StatusReport, WorktreeError> {
    let mut report = StatusReport::default();
    let root_path = root.as_ref();

    let disk_files = scan_worktree(root_path, ignore)?;

    let mut head_files = BTreeMap::new();
    if let Some(tree_id) = head_tree_id {
        flatten_tree(store, tree_id, "", &mut head_files)?;
    }

    // 1. Evaluate Staged Changes (Index vs HEAD tree)
    for entry in index.iter() {
        match head_files.get(&entry.path) {
            Some((_, head_oid)) => {
                if head_oid != &entry.object_id {
                    report.staged_modified.push(entry.path.clone());
                }
            }
            None => {
                report.staged_new.push(entry.path.clone());
            }
        }
    }

    for head_path in head_files.keys() {
        if index.get(head_path).is_none() {
            report.staged_deleted.push(head_path.clone());
        }
    }

    // 2. Evaluate Unstaged Changes (Disk vs Index)
    for entry in index.iter() {
        match disk_files.get(&entry.path) {
            Some(disk_abs_path) => {
                if let Ok(meta) = fs::metadata(disk_abs_path) {
                    let (_, size, mtime_secs, mtime_nanos) = metadata_to_stat(&meta);
                    // Fast stat check
                    if size != entry.file_size
                        || mtime_secs != entry.mtime_secs
                        || mtime_nanos != entry.mtime_nanos
                    {
                        // File modified on disk compared to indexed version
                        report.unstaged_modified.push(entry.path.clone());
                    }
                }
            }
            None => {
                // File tracked in index but missing from disk
                report.unstaged_deleted.push(entry.path.clone());
            }
        }
    }

    // 3. Evaluate Untracked Files (Disk vs Index)
    for disk_rel_path in disk_files.keys() {
        if index.get(disk_rel_path).is_none() {
            report.untracked.push(disk_rel_path.clone());
        }
    }

    report.staged_new.sort();
    report.staged_modified.sort();
    report.staged_deleted.sort();
    report.unstaged_modified.sort();
    report.unstaged_deleted.sort();
    report.untracked.sort();

    Ok(report)
}

/// Builds a hierarchical Merkle Tree structure from index entries and writes them to the ObjectStore.
/// Returns the root tree's `ObjectId`.
pub fn write_index_to_tree<S: ObjectStore>(
    index: &Index,
    store: &S,
) -> Result<ObjectId, WorktreeError> {
    let mut tree_map: BTreeMap<String, Vec<TreeEntry>> = BTreeMap::new();

    // Map files to their parent directories
    for entry in index.iter() {
        let parts: Vec<&str> = entry.path.split('/').collect();
        if parts.len() == 1 {
            tree_map.entry("".into()).or_default().push(TreeEntry {
                mode: entry.mode,
                name: parts[0].to_string(),
                object_id: entry.object_id,
            });
        } else {
            // Nested path: e.g. "src/sub/mod.rs"
            let dir_parts = &parts[..parts.len() - 1];
            let dir = dir_parts.join("/");
            let filename = parts[parts.len() - 1];

            tree_map.entry(dir).or_default().push(TreeEntry {
                mode: entry.mode,
                name: filename.to_string(),
                object_id: entry.object_id,
            });

            // Ensure intermediate directories exist in tree_map
            for i in 0..dir_parts.len() {
                let parent_dir = if i == 0 {
                    "".to_string()
                } else {
                    dir_parts[..i].join("/")
                };
                tree_map.entry(parent_dir).or_default();
            }
        }
    }

    // Bottom-up traversal: process directories sorted descending by path length
    // (guarantees all subdirectories are processed before their parents)
    let mut dirs: Vec<String> = tree_map.keys().cloned().collect();
    dirs.sort_by_key(|b| std::cmp::Reverse(b.len()));

    let mut written_subtrees: BTreeMap<String, ObjectId> = BTreeMap::new();

    for dir in dirs {
        let mut entries = tree_map.remove(&dir).unwrap_or_default();

        // Check if any child subtrees belong to this dir
        for (subtree_path, subtree_oid) in &written_subtrees {
            let (parent_path, child_name) = match subtree_path.rfind('/') {
                Some(idx) => (&subtree_path[..idx], &subtree_path[idx + 1..]),
                None => ("", subtree_path.as_str()),
            };
            if parent_path == dir {
                entries.push(TreeEntry {
                    mode: FileMode::Directory,
                    name: child_name.to_string(),
                    object_id: *subtree_oid,
                });
            }
        }

        let serialized = spine_format::serialize_tree(&entries);
        let tree_oid = store.write_object(ObjectType::Tree, &serialized)?;
        written_subtrees.insert(dir, tree_oid);
    }

    let root_oid = written_subtrees.get("").copied().unwrap_or_else(|| {
        // Empty tree
        let empty_serialized = spine_format::serialize_tree(&[]);
        store
            .write_object(ObjectType::Tree, &empty_serialized)
            .unwrap()
    });

    Ok(root_oid)
}
