use crate::{
    compute_status, execute_switch, flatten_tree, scan_worktree, write_index_to_tree, IgnoreRules,
    WorktreeError,
};
use serde::{Deserialize, Serialize};
use spine_core::{Commit, ObjectId, ObjectType};
use spine_format::{deserialize_commit, serialize_commit};
use spine_index::{Index, IndexEntry, StageStatus};
use spine_object_store::ObjectStore;
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashEntry {
    pub index: usize,
    pub commit_id: ObjectId,
    pub worktree_tree: ObjectId,
    pub index_tree: ObjectId,
    pub untracked_tree: Option<ObjectId>,
    pub branch: String,
    pub message: String,
    pub timestamp_nanos: i64,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashList {
    pub entries: Vec<StashEntry>,
}

impl StashList {
    pub fn file_path<P: AsRef<Path>>(spine_dir: P) -> std::path::PathBuf {
        spine_dir.as_ref().join("stash_list.json")
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> io::Result<Self> {
        let path = Self::file_path(spine_dir);
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = fs::read_to_string(path)?;
        let list: Self = serde_json::from_str(&data)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        Ok(list)
    }

    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> io::Result<()> {
        let path = Self::file_path(spine_dir);
        let data = serde_json::to_string_pretty(self)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
        fs::write(path, data)?;
        Ok(())
    }

    pub fn push_entry(&mut self, mut entry: StashEntry) {
        entry.index = 0;
        for (i, e) in self.entries.iter_mut().enumerate() {
            e.index = i + 1;
        }
        self.entries.insert(0, entry);
    }

    pub fn remove_entry(&mut self, index: usize) -> Option<StashEntry> {
        let pos = self.entries.iter().position(|e| e.index == index)?;
        let removed = self.entries.remove(pos);
        for (i, e) in self.entries.iter_mut().enumerate() {
            e.index = i;
        }
        Some(removed)
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn get(&self, index: usize) -> Option<&StashEntry> {
        self.entries.iter().find(|e| e.index == index)
    }
}

/// Executes a `stash push`: snapshot dirty working tree, staged index, and optionally untracked files into a stash commit, then resets worktree to HEAD.
#[allow(clippy::too_many_arguments)]
pub fn stash_push<P: AsRef<Path>, Q: AsRef<Path>, S: ObjectStore>(
    root: P,
    spine_dir: Q,
    index: &mut Index,
    store: &S,
    head_commit_id: Option<&ObjectId>,
    branch_name: &str,
    custom_message: Option<&str>,
    include_untracked: bool,
    ignore: &IgnoreRules,
) -> Result<Option<StashEntry>, WorktreeError> {
    let root_path = root.as_ref();
    let spine_path = spine_dir.as_ref();

    let head_tree_id = if let Some(hc) = head_commit_id {
        let (_, c_bytes) = store.read_object(hc)?;
        let commit = deserialize_commit(&c_bytes)?;
        Some(commit.root_tree)
    } else {
        None
    };

    let status = compute_status(root_path, index, store, head_tree_id.as_ref(), ignore)?;
    let has_staged = !status.staged_new.is_empty()
        || !status.staged_modified.is_empty()
        || !status.staged_deleted.is_empty();
    let has_unstaged = !status.unstaged_modified.is_empty() || !status.unstaged_deleted.is_empty();
    let has_untracked = include_untracked && !status.untracked.is_empty();

    if !has_staged && !has_unstaged && !has_untracked {
        return Ok(None);
    }

    // 1. Staged index tree
    let index_tree = write_index_to_tree(index, store)?;

    // 2. Build worktree dirty tree (staged + unstaged modifications on disk)
    let mut dirty_index = index.clone();
    let disk_files = scan_worktree(root_path, ignore)?;
    for (rel_path, abs_path) in &disk_files {
        if dirty_index.get(rel_path).is_some() {
            // Update blob in dirty_index with current disk content
            let content = fs::read(abs_path)?;
            let blob_id = store.write_object(ObjectType::Blob, &content)?;
            let meta = fs::metadata(abs_path)?;
            let (mode, size, mtime_secs, mtime_nanos) = crate::metadata_to_stat(&meta);
            dirty_index.add_or_update(IndexEntry {
                path: rel_path.clone(),
                mode,
                file_size: size,
                mtime_secs,
                mtime_nanos,
                inode: 0,
                object_id: blob_id,
                stage_status: StageStatus::Clean,
            });
        }
    }
    // Remove files deleted on disk
    for unstaged_del in &status.unstaged_deleted {
        dirty_index.remove(unstaged_del);
    }
    let worktree_tree = write_index_to_tree(&dirty_index, store)?;

    // 3. Optional untracked files tree
    let untracked_tree = if has_untracked {
        let mut untracked_index = Index::new();
        for rel_untracked in &status.untracked {
            if let Some(abs_path) = disk_files.get(rel_untracked) {
                let content = fs::read(abs_path)?;
                let blob_id = store.write_object(ObjectType::Blob, &content)?;
                let meta = fs::metadata(abs_path)?;
                let (mode, size, mtime_secs, mtime_nanos) = crate::metadata_to_stat(&meta);
                untracked_index.add_or_update(IndexEntry {
                    path: rel_untracked.clone(),
                    mode,
                    file_size: size,
                    mtime_secs,
                    mtime_nanos,
                    inode: 0,
                    object_id: blob_id,
                    stage_status: StageStatus::Clean,
                });
            }
        }
        Some(write_index_to_tree(&untracked_index, store)?)
    } else {
        None
    };

    // 4. Stash commit
    let mut parents = Vec::new();
    if let Some(hc) = head_commit_id {
        parents.push(*hc);
    }
    // Parent 2: commit for staged tree
    let staged_commit = Commit::new_simple(
        index_tree,
        parents.clone(),
        "Stash Index".into(),
        "stash@spine.org".into(),
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        "index on stash".into(),
    );
    let staged_cid = store.write_object(ObjectType::Commit, &serialize_commit(&staged_commit))?;
    parents.push(staged_cid);

    if let Some(utree) = untracked_tree {
        let untracked_commit = Commit::new_simple(
            utree,
            vec![],
            "Stash Untracked".into(),
            "stash@spine.org".into(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
            "untracked files on stash".into(),
        );
        let u_cid = store.write_object(ObjectType::Commit, &serialize_commit(&untracked_commit))?;
        parents.push(u_cid);
    }

    let head_short = head_commit_id
        .map(|h| h.to_hex()[..7].to_string())
        .unwrap_or_else(|| "0000000".to_string());
    let default_msg = format!("WIP on {}: {}", branch_name, head_short);
    let message = custom_message.unwrap_or(&default_msg).to_string();
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    let stash_commit = Commit::new_simple(
        worktree_tree,
        parents,
        "Spine Stash".into(),
        "stash@spine.org".into(),
        now,
        message.clone(),
    );
    let stash_commit_id =
        store.write_object(ObjectType::Commit, &serialize_commit(&stash_commit))?;

    // 5. Reset working tree to HEAD
    if let Some(ht_id) = head_tree_id {
        execute_switch(root_path, index, store, Some(&worktree_tree), &ht_id)?;
    } else {
        // Clean all files if no HEAD commit
        for entry in index.iter() {
            let p = root_path.join(&entry.path);
            if p.exists() {
                let _ = fs::remove_file(p);
            }
        }
        index.clear();
    }

    // Clean untracked files that were stashed
    if has_untracked {
        for untracked in &status.untracked {
            let p = root_path.join(untracked);
            if p.exists() {
                let _ = fs::remove_file(&p);
                let _ = crate::remove_empty_parent_dirs(root_path, untracked);
            }
        }
    }

    // 6. Record in StashList
    let mut list = StashList::load_from_dir(spine_path)?;
    let entry = StashEntry {
        index: 0,
        commit_id: stash_commit_id,
        worktree_tree,
        index_tree,
        untracked_tree,
        branch: branch_name.to_string(),
        message,
        timestamp_nanos: now,
    };
    list.push_entry(entry.clone());
    list.save_to_dir(spine_path)?;

    // Update ref .spine/refs/stash
    let refs_stash = spine_path.join("refs").join("stash");
    if let Some(parent) = refs_stash.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(refs_stash, stash_commit_id.to_hex());

    Ok(Some(entry))
}

/// Applies a stash entry to the working directory and staging index.
pub fn stash_apply<P: AsRef<Path>, Q: AsRef<Path>, S: ObjectStore>(
    root: P,
    spine_dir: Q,
    index: &mut Index,
    store: &S,
    stash_idx: usize,
    reinstate_index: bool,
) -> Result<StashEntry, WorktreeError> {
    let root_path = root.as_ref();
    let spine_path = spine_dir.as_ref();

    let list = StashList::load_from_dir(spine_path)?;
    let entry = list
        .get(stash_idx)
        .cloned()
        .ok_or_else(|| WorktreeError::FileNotFound(format!("stash@{{{}}}", stash_idx)))?;

    // Restore worktree files from worktree_tree
    let mut worktree_files = BTreeMap::new();
    flatten_tree(store, &entry.worktree_tree, "", &mut worktree_files)?;

    for (rel_path, (mode, oid)) in worktree_files {
        let abs_path = root_path.join(&rel_path);
        if let Some(parent) = abs_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let (_, content) = store.read_object(&oid)?;
        fs::write(&abs_path, &content)?;

        let meta = fs::metadata(&abs_path)?;
        let (_, size, mtime_secs, mtime_nanos) = crate::metadata_to_stat(&meta);

        index.add_or_update(IndexEntry {
            path: rel_path,
            mode,
            file_size: size,
            mtime_secs,
            mtime_nanos,
            inode: 0,
            object_id: oid,
            stage_status: StageStatus::Clean,
        });
    }

    // Restore untracked files if present
    if let Some(utree_id) = entry.untracked_tree {
        let mut untracked_files = BTreeMap::new();
        flatten_tree(store, &utree_id, "", &mut untracked_files)?;
        for (rel_path, (_, oid)) in untracked_files {
            let abs_path = root_path.join(&rel_path);
            if let Some(parent) = abs_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let (_, content) = store.read_object(&oid)?;
            fs::write(&abs_path, &content)?;
        }
    }

    // If reinstate_index: restore staging index to match entry.index_tree
    if reinstate_index {
        let mut index_files = BTreeMap::new();
        flatten_tree(store, &entry.index_tree, "", &mut index_files)?;
        index.clear();
        for (rel_path, (mode, oid)) in index_files {
            let abs_path = root_path.join(&rel_path);
            if let Ok(meta) = fs::metadata(&abs_path) {
                let (_, size, mtime_secs, mtime_nanos) = crate::metadata_to_stat(&meta);
                index.add_or_update(IndexEntry {
                    path: rel_path,
                    mode,
                    file_size: size,
                    mtime_secs,
                    mtime_nanos,
                    inode: 0,
                    object_id: oid,
                    stage_status: StageStatus::Staged,
                });
            }
        }
    }

    Ok(entry)
}

/// Pops a stash entry: applies it and then drops it from the stash stack.
pub fn stash_pop<P: AsRef<Path>, Q: AsRef<Path>, S: ObjectStore>(
    root: P,
    spine_dir: Q,
    index: &mut Index,
    store: &S,
    stash_idx: usize,
    reinstate_index: bool,
) -> Result<StashEntry, WorktreeError> {
    let entry = stash_apply(
        root,
        spine_dir.as_ref(),
        index,
        store,
        stash_idx,
        reinstate_index,
    )?;
    let mut list = StashList::load_from_dir(spine_dir.as_ref())?;
    list.remove_entry(stash_idx);
    list.save_to_dir(spine_dir.as_ref())?;

    // Update ref .spine/refs/stash to top of stack or remove if empty
    let refs_stash = spine_dir.as_ref().join("refs").join("stash");
    if let Some(top) = list.entries.first() {
        let _ = fs::write(refs_stash, top.commit_id.to_hex());
    } else if refs_stash.exists() {
        let _ = fs::remove_file(refs_stash);
    }

    Ok(entry)
}

/// Drops a stash entry without applying it.
pub fn stash_drop<P: AsRef<Path>>(
    spine_dir: P,
    stash_idx: usize,
) -> Result<StashEntry, WorktreeError> {
    let mut list = StashList::load_from_dir(spine_dir.as_ref())?;
    let entry = list
        .remove_entry(stash_idx)
        .ok_or_else(|| WorktreeError::FileNotFound(format!("stash@{{{}}}", stash_idx)))?;
    list.save_to_dir(spine_dir.as_ref())?;

    let refs_stash = spine_dir.as_ref().join("refs").join("stash");
    if let Some(top) = list.entries.first() {
        let _ = fs::write(refs_stash, top.commit_id.to_hex());
    } else if refs_stash.exists() {
        let _ = fs::remove_file(refs_stash);
    }

    Ok(entry)
}

/// Clears the entire stash stack.
pub fn stash_clear<P: AsRef<Path>>(spine_dir: P) -> Result<(), WorktreeError> {
    let mut list = StashList::load_from_dir(spine_dir.as_ref())?;
    list.clear();
    list.save_to_dir(spine_dir.as_ref())?;

    let refs_stash = spine_dir.as_ref().join("refs").join("stash");
    if refs_stash.exists() {
        let _ = fs::remove_file(refs_stash);
    }
    Ok(())
}
