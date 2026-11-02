//! # spine-worktree
//!
//! Working tree traversal, ignore rules evaluation, stat-cache diffing,
//! and staging mechanics for Spine repositories.

pub mod multi;
pub mod stash;

pub use multi::{
    add_worktree, get_checked_out_branches, list_worktrees, prune_worktrees, remove_worktree,
    resolve_worktree_context, WorktreeInfo, WorktreeMetadata,
};
pub use stash::{
    stash_apply, stash_clear, stash_drop, stash_pop, stash_push, StashEntry, StashList,
};

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
    #[error("Your local changes to the following files would be overwritten by switch:\n  {0}")]
    LocalChangesWouldBeOverwritten(String),
    #[error("The following untracked working tree files would be overwritten by switch:\n  {0}")]
    UntrackedWouldBeOverwritten(String),
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

/// Preflight safety check before switching the working tree.
/// Verifies that no uncommitted changes (staged or unstaged) or untracked files
/// would be overwritten by the files that differ between the current tree and target tree.
pub fn check_switch_safety<P: AsRef<Path>, S: ObjectStore>(
    root: P,
    index: &Index,
    store: &S,
    current_tree_id: Option<&ObjectId>,
    target_tree_id: &ObjectId,
    ignore: &IgnoreRules,
) -> Result<(), WorktreeError> {
    let root_path = root.as_ref();
    let status = compute_status(root_path, index, store, current_tree_id, ignore)?;

    let mut current_files = BTreeMap::new();
    if let Some(cur_id) = current_tree_id {
        flatten_tree(store, cur_id, "", &mut current_files)?;
    }

    let mut target_files = BTreeMap::new();
    flatten_tree(store, target_tree_id, "", &mut target_files)?;

    // Collect all paths that differ between current tree and target tree
    let mut touched_by_switch = std::collections::BTreeSet::new();
    for (path, (mode, oid)) in &target_files {
        match current_files.get(path) {
            Some((c_mode, c_oid)) => {
                if mode != c_mode || oid != c_oid {
                    touched_by_switch.insert(path.clone());
                }
            }
            None => {
                touched_by_switch.insert(path.clone());
            }
        }
    }
    for path in current_files.keys() {
        if !target_files.contains_key(path) {
            touched_by_switch.insert(path.clone());
        }
    }

    // Check untracked files:
    // If an untracked file would be overwritten by a file introduced or changed by target tree
    for untracked in &status.untracked {
        if target_files.contains_key(untracked) {
            return Err(WorktreeError::UntrackedWouldBeOverwritten(
                untracked.clone(),
            ));
        }
    }

    // Check uncommitted changes:
    let mut dirty_paths = std::collections::BTreeSet::new();
    dirty_paths.extend(status.staged_new);
    dirty_paths.extend(status.staged_modified);
    dirty_paths.extend(status.staged_deleted);
    dirty_paths.extend(status.unstaged_modified);
    dirty_paths.extend(status.unstaged_deleted);

    for dirty in dirty_paths {
        if touched_by_switch.contains(&dirty) {
            return Err(WorktreeError::LocalChangesWouldBeOverwritten(dirty));
        }
    }

    Ok(())
}

/// Executes the working tree switch: updates disk files, cleans deleted files & empty directories,
/// and updates the staging index to match the target commit tree.
pub fn execute_switch<P: AsRef<Path>, S: ObjectStore>(
    root: P,
    index: &mut Index,
    store: &S,
    current_tree_id: Option<&ObjectId>,
    target_tree_id: &ObjectId,
) -> Result<(), WorktreeError> {
    let root_path = root.as_ref();

    let mut current_files = BTreeMap::new();
    if let Some(cur_id) = current_tree_id {
        flatten_tree(store, cur_id, "", &mut current_files)?;
    }

    let mut target_files = BTreeMap::new();
    flatten_tree(store, target_tree_id, "", &mut target_files)?;

    // 1. Remove files in current that are not in target
    for path in current_files.keys() {
        if !target_files.contains_key(path) {
            let file_path = root_path.join(path);
            if file_path.exists() {
                fs::remove_file(&file_path)?;
                let _ = remove_empty_parent_dirs(root_path, path);
            }
            index.remove(path);
        }
    }

    // 2. Add or update files from target
    for (path, (mode, oid)) in &target_files {
        let needs_update = match current_files.get(path) {
            Some((c_mode, c_oid)) => c_mode != mode || c_oid != oid,
            None => true,
        };

        let file_path = root_path.join(path);
        if needs_update || !file_path.exists() {
            if let Some(parent) = file_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let (_, content) = store.read_object(oid)?;
            fs::write(&file_path, &content)?;
        }

        // Update index entry
        let meta = fs::metadata(&file_path)?;
        let (file_mode, size, mtime_secs, mtime_nanos) = metadata_to_stat(&meta);

        index.add_or_update(IndexEntry {
            path: path.clone(),
            mode: file_mode,
            file_size: size,
            mtime_secs,
            mtime_nanos,
            inode: 0,
            object_id: *oid,
            stage_status: StageStatus::Clean,
        });
    }

    Ok(())
}

fn remove_empty_parent_dirs(root: &Path, rel_file_path: &str) -> io::Result<()> {
    let mut current = root.join(rel_file_path);
    while let Some(parent) = current.parent() {
        if parent == root || !parent.starts_with(root) {
            break;
        }
        match fs::read_dir(parent) {
            Ok(mut read) => {
                if read.next().is_none() {
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

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::{Commit, HashAlgorithm};
    use spine_format::serialize_commit;
    use spine_object_store::MemoryObjectStore;
    use tempfile::tempdir;

    #[test]
    fn test_stash_push_and_pop_lifecycle() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("repo");
        let spine_dir = root.join(".spine");
        fs::create_dir_all(&spine_dir).unwrap();

        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);
        let mut index = Index::new();
        let ignore = IgnoreRules::default();

        // Create base commit with file.txt = "base"
        let b0 = store.write_object(ObjectType::Blob, b"base").unwrap();
        let t0 = store
            .write_object(
                ObjectType::Tree,
                &spine_format::serialize_tree(&[TreeEntry {
                    mode: FileMode::Regular,
                    name: "file.txt".into(),
                    object_id: b0,
                }]),
            )
            .unwrap();
        let c0 = Commit::new_simple(
            t0,
            vec![],
            "Author".into(),
            "a@b.com".into(),
            100,
            "Initial".into(),
        );
        let c0_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c0))
            .unwrap();

        // Working tree has file.txt on disk
        fs::write(root.join("file.txt"), "modified on disk").unwrap();
        index.add_or_update(IndexEntry {
            path: "file.txt".into(),
            mode: FileMode::Regular,
            file_size: 4,
            mtime_secs: 100,
            mtime_nanos: 0,
            inode: 0,
            object_id: b0,
            stage_status: StageStatus::Clean,
        });

        // Untracked file
        fs::write(root.join("untracked.txt"), "hello untracked").unwrap();

        // Stash push with untracked
        let stashed = stash_push(
            &root,
            &spine_dir,
            &mut index,
            &store,
            Some(&c0_id),
            "main",
            None,
            true,
            &ignore,
        )
        .unwrap();

        assert!(stashed.is_some());
        // Verify working tree is restored to clean base
        let content_after = fs::read_to_string(root.join("file.txt")).unwrap();
        assert_eq!(content_after, "base");
        assert!(!root.join("untracked.txt").exists());

        // Stash pop
        let popped = stash_pop(&root, &spine_dir, &mut index, &store, 0, false).unwrap();
        assert_eq!(popped.commit_id, stashed.unwrap().commit_id);

        let content_popped = fs::read_to_string(root.join("file.txt")).unwrap();
        assert_eq!(content_popped, "modified on disk");
        assert!(root.join("untracked.txt").exists());
        assert_eq!(
            fs::read_to_string(root.join("untracked.txt")).unwrap(),
            "hello untracked"
        );
    }

    #[test]
    fn test_multi_worktree_lifecycle_and_branch_locking() {
        let tmp = tempdir().unwrap();
        let root = tmp.path().join("repo");
        let spine_dir = root.join(".spine");
        fs::create_dir_all(spine_dir.join("refs").join("heads")).unwrap();
        fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();

        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // Commit C0
        let b0 = store.write_object(ObjectType::Blob, b"primary").unwrap();
        let t0 = store
            .write_object(
                ObjectType::Tree,
                &spine_format::serialize_tree(&[TreeEntry {
                    mode: FileMode::Regular,
                    name: "hello.txt".into(),
                    object_id: b0,
                }]),
            )
            .unwrap();
        let c0 = Commit::new_simple(
            t0,
            vec![],
            "Author".into(),
            "a@b.com".into(),
            100,
            "Initial".into(),
        );
        let c0_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c0))
            .unwrap();
        fs::write(
            spine_dir.join("refs").join("heads").join("main"),
            c0_id.to_hex(),
        )
        .unwrap();

        // 1. Cannot add worktree with branch 'main' because it's checked out in primary
        let wt_path = tmp.path().join("linked_feature");
        let conflict = add_worktree(&spine_dir, &wt_path, Some("main"), &c0_id, &store);
        assert!(conflict.is_err());
        assert!(conflict
            .unwrap_err()
            .to_string()
            .contains("already checked out"));

        // 2. Add worktree with branch 'feature'
        let wt = add_worktree(&spine_dir, &wt_path, Some("feature"), &c0_id, &store).unwrap();
        assert_eq!(wt.branch, Some("feature".into()));
        assert!(wt_path.join(".spine").is_file());
        assert!(wt_path.join("hello.txt").is_file());

        // 3. Resolve context inside linked worktree
        let (wt_root, eff_spine, prim_spine, is_linked) =
            resolve_worktree_context(&wt_path).unwrap();
        assert!(is_linked);
        assert_eq!(wt_root, wt.path);
        assert_eq!(
            prim_spine,
            multi::normalize_canonical_path(spine_dir.canonicalize().unwrap())
        );
        assert!(eff_spine.ends_with(&wt.id));

        // 4. List worktrees shows 2 entries (primary + feature)
        let list = list_worktrees(&spine_dir).unwrap();
        assert_eq!(list.len(), 2);
        assert!(list[0].is_primary);
        assert!(!list[1].is_primary);

        // 5. Remove worktree
        remove_worktree(&spine_dir, &wt.id, false).unwrap();
        let list_after = list_worktrees(&spine_dir).unwrap();
        assert_eq!(list_after.len(), 1);
        assert!(!wt_path.exists());
    }
}
