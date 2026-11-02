use crate::{execute_switch, WorktreeError};
use serde::{Deserialize, Serialize};
use spine_core::ObjectId;
use spine_format::deserialize_commit;
use spine_index::Index;
use spine_object_store::ObjectStore;
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeMetadata {
    pub id: String,
    pub path: String,
    pub branch: Option<String>,
    pub locked: bool,
    pub lock_reason: Option<String>,
    pub created_at_nanos: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeInfo {
    pub id: String,
    pub path: PathBuf,
    pub head_oid: Option<ObjectId>,
    pub branch: Option<String>,
    pub is_primary: bool,
    pub is_prunable: bool,
    pub locked: bool,
    pub lock_reason: Option<String>,
}

/// Normalizes paths by removing Windows extended-length path prefix (`\\?\`) if present.
pub fn normalize_canonical_path<P: AsRef<Path>>(path: P) -> PathBuf {
    let p = path.as_ref();
    let s = p.to_string_lossy();
    if let Some(stripped) = s.strip_prefix(r"\\?\") {
        PathBuf::from(stripped)
    } else {
        p.to_path_buf()
    }
}

/// Resolves whether a path is inside a primary repository or a linked worktree.
/// Returns `(worktree_root, effective_spine_dir, primary_spine_dir, is_linked)`.
pub fn resolve_worktree_context<P: AsRef<Path>>(
    start_dir: P,
) -> Result<(PathBuf, PathBuf, PathBuf, bool), WorktreeError> {
    let mut current = start_dir.as_ref().to_path_buf();
    loop {
        let spine_path = current.join(".spine");
        if spine_path.is_dir() {
            // Primary repository
            let norm_current = normalize_canonical_path(&current);
            let norm_spine = normalize_canonical_path(&spine_path);
            return Ok((norm_current, norm_spine.clone(), norm_spine, false));
        } else if spine_path.is_file() {
            // Linked worktree .spine pointer file: "gitdir: <path>"
            let content = fs::read_to_string(&spine_path)?;
            let trimmed = content.trim();
            if let Some(gitdir_str) = trimmed.strip_prefix("gitdir:") {
                let gitdir = normalize_canonical_path(PathBuf::from(gitdir_str.trim()));
                // gitdir is like <primary_spine_dir>/worktrees/<id>
                let primary_spine_dir = gitdir
                    .parent()
                    .and_then(|p| p.parent())
                    .map(|p| normalize_canonical_path(p.to_path_buf()))
                    .unwrap_or_else(|| gitdir.clone());
                return Ok((
                    normalize_canonical_path(&current),
                    gitdir,
                    primary_spine_dir,
                    true,
                ));
            }
        }

        if !current.pop() {
            break;
        }
    }

    Err(WorktreeError::FileNotFound(
        "Not inside a Spine repository or linked worktree".into(),
    ))
}

/// Returns a map of `branch_name -> worktree_id_or_path` for all currently checked-out local branches across primary and all linked worktrees.
pub fn get_checked_out_branches<P: AsRef<Path>>(
    primary_spine_dir: P,
) -> Result<HashMap<String, String>, WorktreeError> {
    let mut checked_out = HashMap::new();
    let prim_dir = primary_spine_dir.as_ref();

    // 1. Check primary worktree HEAD
    let primary_head_file = prim_dir.join("HEAD");
    if primary_head_file.exists() {
        if let Ok(content) = fs::read_to_string(&primary_head_file) {
            let trimmed = content.trim();
            if let Some(branch_ref) = trimmed.strip_prefix("ref: refs/heads/") {
                checked_out.insert(branch_ref.to_string(), "primary".to_string());
            }
        }
    }

    // 2. Check each linked worktree in primary_spine_dir/worktrees/<id>/HEAD
    let worktrees_dir = prim_dir.join("worktrees");
    if worktrees_dir.is_dir() {
        for entry in fs::read_dir(worktrees_dir)? {
            let entry = entry?;
            let wt_dir = entry.path();
            if wt_dir.is_dir() {
                let wt_id = entry.file_name().to_string_lossy().to_string();
                let head_file = wt_dir.join("HEAD");
                if head_file.exists() {
                    if let Ok(content) = fs::read_to_string(&head_file) {
                        let trimmed = content.trim();
                        if let Some(branch_ref) = trimmed.strip_prefix("ref: refs/heads/") {
                            checked_out.insert(branch_ref.to_string(), wt_id);
                        }
                    }
                }
            }
        }
    }

    Ok(checked_out)
}

/// Adds a new linked worktree at `target_path` checking out `branch` (or `commit_id`).
pub fn add_worktree<P: AsRef<Path>, S: ObjectStore>(
    primary_spine_dir: P,
    target_path: P,
    branch: Option<&str>,
    commit_id: &ObjectId,
    store: &S,
) -> Result<WorktreeInfo, WorktreeError> {
    let prim_spine = primary_spine_dir.as_ref();
    let target = target_path.as_ref();

    if target.exists() && target.read_dir()?.next().is_some() {
        return Err(WorktreeError::UnsafePath(format!(
            "Target directory '{}' already exists and is not empty",
            target.display()
        )));
    }

    // Branch ownership check
    if let Some(b) = branch {
        let active_branches = get_checked_out_branches(prim_spine)?;
        if let Some(owner) = active_branches.get(b) {
            return Err(WorktreeError::UnsafePath(format!(
                "Fatal: branch '{}' is already checked out in worktree '{}'",
                b, owner
            )));
        }
    }

    // Generate unique ID based on target directory name
    let dir_name = target
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("worktree");
    let wt_id = format!("{}-{}", dir_name, &commit_id.to_hex()[..6]);

    let wt_spine_dir = prim_spine.join("worktrees").join(&wt_id);
    fs::create_dir_all(&wt_spine_dir)?;

    // Write linked HEAD
    let head_content = if let Some(b) = branch {
        let branch_ref_path = prim_spine.join("refs").join("heads").join(b);
        if !branch_ref_path.exists() {
            if let Some(parent) = branch_ref_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&branch_ref_path, format!("{}\n", commit_id.to_hex()))?;
        }
        format!("ref: refs/heads/{}\n", b)
    } else {
        format!("{}\n", commit_id.to_hex())
    };
    fs::write(wt_spine_dir.join("HEAD"), &head_content)?;

    // Write empty or clean index
    let mut index = Index::new();
    let index_bytes = index.serialize();
    fs::write(wt_spine_dir.join("index"), &index_bytes)?;

    // Write worktree.json metadata
    let canonical_target = if target.exists() {
        normalize_canonical_path(
            target
                .canonicalize()
                .unwrap_or_else(|_| target.to_path_buf()),
        )
    } else {
        fs::create_dir_all(target)?;
        normalize_canonical_path(
            target
                .canonicalize()
                .unwrap_or_else(|_| target.to_path_buf()),
        )
    };

    let meta = WorktreeMetadata {
        id: wt_id.clone(),
        path: canonical_target.to_string_lossy().to_string(),
        branch: branch.map(String::from),
        locked: false,
        lock_reason: None,
        created_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
    };
    let meta_json = serde_json::to_string_pretty(&meta)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
    fs::write(wt_spine_dir.join("worktree.json"), meta_json)?;

    // Write .spine pointer file into linked worktree
    let canonical_wt_spine = normalize_canonical_path(
        wt_spine_dir
            .canonicalize()
            .unwrap_or_else(|_| wt_spine_dir.clone()),
    );
    let pointer_content = format!("gitdir: {}\n", canonical_wt_spine.to_string_lossy());
    fs::write(canonical_target.join(".spine"), pointer_content)?;

    // Populate worktree with files from commit_id root tree
    let (_, c_bytes) = store.read_object(commit_id)?;
    let commit = deserialize_commit(&c_bytes)?;
    execute_switch(
        &canonical_target,
        &mut index,
        store,
        None,
        &commit.root_tree,
    )?;

    // Save updated index to wt_spine_dir
    let final_index_bytes = index.serialize();
    fs::write(wt_spine_dir.join("index"), &final_index_bytes)?;

    Ok(WorktreeInfo {
        id: wt_id,
        path: canonical_target,
        head_oid: Some(*commit_id),
        branch: branch.map(String::from),
        is_primary: false,
        is_prunable: false,
        locked: false,
        lock_reason: None,
    })
}

/// Lists all worktrees (primary + all linked worktrees).
pub fn list_worktrees<P: AsRef<Path>>(
    primary_spine_dir: P,
) -> Result<Vec<WorktreeInfo>, WorktreeError> {
    let mut result = Vec::new();
    let prim_spine = primary_spine_dir.as_ref();
    let prim_root = prim_spine
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));

    // 1. Primary worktree info
    let primary_head_file = prim_spine.join("HEAD");
    let mut prim_branch = None;
    let mut prim_oid = None;
    if primary_head_file.exists() {
        if let Ok(c) = fs::read_to_string(&primary_head_file) {
            let trimmed = c.trim();
            if let Some(b) = trimmed.strip_prefix("ref: refs/heads/") {
                prim_branch = Some(b.to_string());
                let b_file = prim_spine.join("refs").join("heads").join(b);
                if let Ok(oid_hex) = fs::read_to_string(b_file) {
                    prim_oid = ObjectId::from_hex(oid_hex.trim()).ok();
                }
            } else {
                prim_oid = ObjectId::from_hex(trimmed).ok();
            }
        }
    }

    result.push(WorktreeInfo {
        id: "primary".to_string(),
        path: prim_root,
        head_oid: prim_oid,
        branch: prim_branch,
        is_primary: true,
        is_prunable: false,
        locked: false,
        lock_reason: None,
    });

    // 2. Linked worktrees
    let worktrees_dir = prim_spine.join("worktrees");
    if worktrees_dir.is_dir() {
        for entry in fs::read_dir(worktrees_dir)? {
            let entry = entry?;
            let wt_dir = entry.path();
            if !wt_dir.is_dir() {
                continue;
            }

            let wt_id = entry.file_name().to_string_lossy().to_string();
            let meta_file = wt_dir.join("worktree.json");
            let mut target_path = PathBuf::from(&wt_id);
            let mut branch = None;
            let mut locked = false;
            let mut lock_reason = None;

            if meta_file.exists() {
                if let Ok(data) = fs::read_to_string(&meta_file) {
                    if let Ok(m) = serde_json::from_str::<WorktreeMetadata>(&data) {
                        target_path = PathBuf::from(m.path);
                        branch = m.branch;
                        locked = m.locked;
                        lock_reason = m.lock_reason;
                    }
                }
            }

            let head_file = wt_dir.join("HEAD");
            let mut wt_oid = None;
            if head_file.exists() {
                if let Ok(c) = fs::read_to_string(&head_file) {
                    let trimmed = c.trim();
                    if let Some(b) = trimmed.strip_prefix("ref: refs/heads/") {
                        if branch.is_none() {
                            branch = Some(b.to_string());
                        }
                        let b_file = prim_spine.join("refs").join("heads").join(b);
                        if let Ok(oid_hex) = fs::read_to_string(b_file) {
                            wt_oid = ObjectId::from_hex(oid_hex.trim()).ok();
                        }
                    } else {
                        wt_oid = ObjectId::from_hex(trimmed).ok();
                    }
                }
            }

            let is_prunable = !target_path.exists() || !target_path.join(".spine").is_file();

            result.push(WorktreeInfo {
                id: wt_id,
                path: target_path,
                head_oid: wt_oid,
                branch,
                is_primary: false,
                is_prunable,
                locked,
                lock_reason,
            });
        }
    }

    Ok(result)
}

/// Removes a linked worktree by id or path.
pub fn remove_worktree<P: AsRef<Path>>(
    primary_spine_dir: P,
    id_or_path: &str,
    force: bool,
) -> Result<PathBuf, WorktreeError> {
    let prim_spine = primary_spine_dir.as_ref();
    let all = list_worktrees(prim_spine)?;

    let found = all.iter().find(|w| {
        !w.is_primary
            && (w.id == id_or_path
                || w.path.to_string_lossy() == id_or_path
                || w.path.ends_with(id_or_path))
    });

    let Some(wt) = found else {
        return Err(WorktreeError::FileNotFound(format!(
            "Worktree '{}' not found",
            id_or_path
        )));
    };

    if wt.locked && !force {
        return Err(WorktreeError::UnsafePath(format!(
            "Worktree '{}' is locked: {}",
            wt.id,
            wt.lock_reason.as_deref().unwrap_or("no reason given")
        )));
    }

    // Remove the target directory if it exists
    if wt.path.exists() {
        fs::remove_dir_all(&wt.path)?;
    }

    // Remove the .spine/worktrees/<id> metadata directory
    let meta_dir = prim_spine.join("worktrees").join(&wt.id);
    if meta_dir.exists() {
        fs::remove_dir_all(meta_dir)?;
    }

    Ok(wt.path.clone())
}

/// Prunes worktree records whose disk directories no longer exist.
pub fn prune_worktrees<P: AsRef<Path>>(primary_spine_dir: P) -> Result<Vec<String>, WorktreeError> {
    let prim_spine = primary_spine_dir.as_ref();
    let all = list_worktrees(prim_spine)?;
    let mut pruned = Vec::new();

    for wt in all {
        if !wt.is_primary && wt.is_prunable && !wt.locked {
            let meta_dir = prim_spine.join("worktrees").join(&wt.id);
            if meta_dir.exists() {
                fs::remove_dir_all(meta_dir)?;
                pruned.push(wt.id);
            }
        }
    }

    Ok(pruned)
}
