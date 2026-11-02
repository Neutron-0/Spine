use anyhow::{bail, Result};
use chrono::{TimeZone, Utc};
use spine_core::{Commit, FileMode, ObjectId, ObjectType};
use spine_format::serialize_commit;
use spine_history::{
    compute_blame, find_bisect_midpoint, find_commits_between, get_commit, BisectState,
};
use spine_index::{Index, IndexEntry, StageStatus};
use spine_journal::{Journal, OperationKind};
use spine_merge::{
    merge_3way_trees, CherryPickState, MergeState, RebaseState, ResolvedPath, RevertState,
};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use spine_worktree::{
    check_switch_safety, execute_switch, flatten_tree, normalize_rel_path, write_index_to_tree,
    IgnoreRules,
};
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

pub fn cmd_reset(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    target_oid: &ObjectId,
    soft: bool,
    mixed: bool,
    hard: bool,
    paths: &[PathBuf],
    ignore: &IgnoreRules,
) -> Result<()> {
    let mode_soft = soft;
    let mode_hard = hard;
    let _mode_mixed = mixed || (!soft && !hard);

    let target_commit = get_commit(store, target_oid)?;

    // 1. Path-specific reset
    if !paths.is_empty() {
        let mut target_files = BTreeMap::new();
        flatten_tree(store, &target_commit.root_tree, "", &mut target_files)?;

        for path_arg in paths {
            let abs_path = if path_arg.is_absolute() {
                path_arg.clone()
            } else {
                worktree_root.join(path_arg)
            };

            let rel = match abs_path.strip_prefix(worktree_root) {
                Ok(r) => r,
                Err(_) => bail!("fatal: path '{}' outside repository", path_arg.display()),
            };
            let norm = normalize_rel_path(rel)?;

            if norm.is_empty() || norm == "." {
                // Reset entire index to target commit
                index.clear();
                for (p, (mode, oid)) in &target_files {
                    let disk_path = worktree_root.join(p);
                    let (size, mtime_secs, mtime_nanos) = if let Ok(meta) = fs::metadata(&disk_path)
                    {
                        let (_, s, s_sec, s_nano) = spine_worktree::metadata_to_stat(&meta);
                        (s, s_sec, s_nano)
                    } else {
                        (0, 0, 0)
                    };
                    index.add_or_update(IndexEntry {
                        path: p.clone(),
                        mode: *mode,
                        file_size: size,
                        mtime_secs,
                        mtime_nanos,
                        inode: 0,
                        object_id: *oid,
                        stage_status: StageStatus::Clean,
                    });
                }
                println!("Unstaged all changes");
                break;
            }

            // Find matching index paths
            let matching: Vec<String> = index
                .iter()
                .filter(|e| e.path == norm || e.path.starts_with(&format!("{}/", norm)))
                .map(|e| e.path.clone())
                .collect();

            if matching.is_empty() && !target_files.contains_key(&norm) {
                bail!("fatal: pathspec '{}' did not match any files", norm);
            }

            for p in &matching {
                if let Some((mode, oid)) = target_files.get(p) {
                    let disk_path = worktree_root.join(p);
                    let (size, mtime_secs, mtime_nanos) = if let Ok(meta) = fs::metadata(&disk_path)
                    {
                        let (_, s, s_sec, s_nano) = spine_worktree::metadata_to_stat(&meta);
                        (s, s_sec, s_nano)
                    } else {
                        (0, 0, 0)
                    };
                    index.add_or_update(IndexEntry {
                        path: p.clone(),
                        mode: *mode,
                        file_size: size,
                        mtime_secs,
                        mtime_nanos,
                        inode: 0,
                        object_id: *oid,
                        stage_status: StageStatus::Clean,
                    });
                } else {
                    index.remove(p);
                }
                println!("Unstaged changes for '{}'", p);
            }
        }

        index.save_to_file(spine_dir.join("index"))?;
        return Ok(());
    }

    // 2. Commit reset (Soft, Mixed, Hard)
    let current_state = crate::resolve_head(spine_dir)?;
    let cur_tree = current_state
        .head_commit_id
        .and_then(|cid| get_commit(store, &cid).ok().map(|c| c.root_tree));

    let mut journal = Journal::new(spine_dir.join("journal"))?;
    let before_snap = crate::capture_snapshot(spine_dir, &current_state, store)?;

    if mode_hard {
        // Preflight safety check
        check_switch_safety(
            worktree_root,
            index,
            store,
            cur_tree.as_ref(),
            &target_commit.root_tree,
            ignore,
        )?;

        // Execute hard switch: disk files, index, and refs
        execute_switch(
            worktree_root,
            index,
            store,
            cur_tree.as_ref(),
            &target_commit.root_tree,
        )?;
        index.save_to_file(spine_dir.join("index"))?;
    } else if !mode_soft {
        // Mixed reset (default): update index to match target commit, leave worktree untouched
        let mut target_files = BTreeMap::new();
        flatten_tree(store, &target_commit.root_tree, "", &mut target_files)?;

        index.clear();
        for (p, (mode, oid)) in target_files {
            let disk_path = worktree_root.join(&p);
            let (size, mtime_secs, mtime_nanos) = if let Ok(meta) = fs::metadata(&disk_path) {
                let (_, s, s_sec, s_nano) = spine_worktree::metadata_to_stat(&meta);
                (s, s_sec, s_nano)
            } else {
                (0, 0, 0)
            };
            index.add_or_update(IndexEntry {
                path: p,
                mode,
                file_size: size,
                mtime_secs,
                mtime_nanos,
                inode: 0,
                object_id: oid,
                stage_status: StageStatus::Clean,
            });
        }
        index.save_to_file(spine_dir.join("index"))?;
    }

    // Update branch ref or HEAD
    if let Some(ref branch) = current_state.head_ref_name {
        crate::update_ref(
            primary_spine_dir,
            &format!("refs/heads/{}", branch),
            target_oid,
        )?;
    } else {
        fs::write(spine_dir.join("HEAD"), format!("{}\n", target_oid.to_hex()))?;
    }

    let new_state = crate::resolve_head(spine_dir)?;
    let after_snap = crate::capture_snapshot(spine_dir, &new_state, store)?;

    let tx = journal.prepare_transaction(
        OperationKind::Reset,
        format!(
            "Reset {} to {}",
            if mode_hard {
                "--hard"
            } else if mode_soft {
                "--soft"
            } else {
                "--mixed"
            },
            &target_oid.to_hex()[..7]
        ),
        before_snap,
        None,
    )?;
    journal.commit_transaction(&tx.id, after_snap)?;

    println!(
        "HEAD is now at {} {}",
        &target_oid.to_hex()[..7],
        target_commit.message.lines().next().unwrap_or("")
    );
    Ok(())
}

pub fn cmd_restore(
    worktree_root: &Path,
    spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    paths: &[PathBuf],
    staged: bool,
    worktree: bool,
    source_oid: Option<ObjectId>,
) -> Result<()> {
    let restore_worktree = worktree || (!staged && !worktree);
    let restore_staged = staged;

    // Load source tree if source given
    let source_files: Option<BTreeMap<String, (FileMode, ObjectId)>> =
        if let Some(soid) = source_oid {
            let commit = get_commit(store, &soid)?;
            let mut files = BTreeMap::new();
            flatten_tree(store, &commit.root_tree, "", &mut files)?;
            Some(files)
        } else {
            None
        };

    let mut restored_count = 0;

    for target in paths {
        let abs_target = if target.is_absolute() {
            target.clone()
        } else {
            worktree_root.join(target)
        };

        let rel = match abs_target.strip_prefix(worktree_root) {
            Ok(r) => r,
            Err(_) => bail!("fatal: path '{}' outside repository", target.display()),
        };
        let norm = normalize_rel_path(rel)?;

        // Find candidate files
        let candidate_paths: Vec<String> = if let Some(ref sfiles) = source_files {
            sfiles
                .keys()
                .filter(|p| *p == &norm || p.starts_with(&format!("{}/", norm)))
                .cloned()
                .collect()
        } else {
            index
                .iter()
                .filter(|e| e.path == norm || e.path.starts_with(&format!("{}/", norm)))
                .map(|e| e.path.clone())
                .collect()
        };

        if candidate_paths.is_empty() {
            bail!(
                "fatal: pathspec '{}' did not match any file(s) known to spine",
                norm
            );
        }

        for path in candidate_paths {
            let (mode, oid) = if let Some(ref sfiles) = source_files {
                match sfiles.get(&path) {
                    Some(m_oid) => *m_oid,
                    None => continue,
                }
            } else {
                match index.get(&path) {
                    Some(entry) => (entry.mode, entry.object_id),
                    None => continue,
                }
            };

            let (_, content) = store.read_object(&oid)?;

            if restore_worktree {
                let disk_path = worktree_root.join(&path);
                if let Some(parent) = disk_path.parent() {
                    fs::create_dir_all(parent)?;
                }
                fs::write(&disk_path, &content)?;
                println!("Restored '{}' in working tree", path);
                restored_count += 1;
            }

            if restore_staged {
                let disk_path = worktree_root.join(&path);
                let (size, mtime_secs, mtime_nanos) = if let Ok(meta) = fs::metadata(&disk_path) {
                    let (_, s, s_sec, s_nano) = spine_worktree::metadata_to_stat(&meta);
                    (s, s_sec, s_nano)
                } else {
                    (content.len() as u64, 0, 0)
                };
                index.add_or_update(IndexEntry {
                    path: path.clone(),
                    mode,
                    file_size: size,
                    mtime_secs,
                    mtime_nanos,
                    inode: 0,
                    object_id: oid,
                    stage_status: StageStatus::Clean,
                });
                println!("Restored '{}' in staging index", path);
                restored_count += 1;
            }
        }
    }

    if restore_staged && restored_count > 0 {
        index.save_to_file(spine_dir.join("index"))?;
    }

    Ok(())
}

pub fn cmd_revert(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    target_oid: Option<ObjectId>,
    mainline: Option<usize>,
    continue_revert: bool,
    abort_revert: bool,
    user_name: &str,
    user_email: &str,
) -> Result<()> {
    if abort_revert {
        let state = RevertState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no revert in progress to abort"))?;

        let orig_commit = get_commit(store, &state.orig_head)?;
        execute_switch(worktree_root, index, store, None, &orig_commit.root_tree)?;
        index.save_to_file(spine_dir.join("index"))?;

        let current_state = crate::resolve_head(spine_dir)?;
        if let Some(ref branch) = current_state.head_ref_name {
            crate::update_ref(
                primary_spine_dir,
                &format!("refs/heads/{}", branch),
                &state.orig_head,
            )?;
        } else {
            fs::write(
                spine_dir.join("HEAD"),
                format!("{}\n", state.orig_head.to_hex()),
            )?;
        }

        RevertState::clear(spine_dir)?;
        println!(
            "Revert aborted: restored HEAD to {}",
            &state.orig_head.to_hex()[..7]
        );
        return Ok(());
    }

    if continue_revert {
        let state = RevertState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no revert in progress to continue"))?;

        // Check if merge conflicts remain
        if let Some(ms) = MergeState::load_from_dir(spine_dir)? {
            if !ms.is_fully_resolved() {
                bail!("fatal: Exiting because of unresolved conflicts. Stage resolved files with 'spine add' before continuing.");
            }
            MergeState::clear(spine_dir)?;
        }

        let new_tree_id = write_index_to_tree(index, store)?;
        let target_commit = get_commit(store, &state.target_commit)?;

        let revert_msg = format!(
            "Revert \"{}\"\n\nThis reverts commit {}.",
            target_commit.message.lines().next().unwrap_or(""),
            state.target_commit.to_hex()
        );

        let new_commit = Commit::new_simple(
            new_tree_id,
            vec![state.orig_head],
            user_name.to_string(),
            user_email.to_string(),
            Utc::now().timestamp_nanos_opt().unwrap_or(0),
            revert_msg,
        );
        let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;

        let current_state = crate::resolve_head(spine_dir)?;
        if let Some(ref branch) = current_state.head_ref_name {
            crate::update_ref(
                primary_spine_dir,
                &format!("refs/heads/{}", branch),
                &new_cid,
            )?;
        } else {
            fs::write(spine_dir.join("HEAD"), format!("{}\n", new_cid.to_hex()))?;
        }

        RevertState::clear(spine_dir)?;
        println!(
            "[{}] Reverted commit {}",
            &new_cid.to_hex()[..7],
            &state.target_commit.to_hex()[..7]
        );
        return Ok(());
    }

    let to_revert_oid =
        target_oid.ok_or_else(|| anyhow::anyhow!("fatal: commit revision required for revert"))?;
    let to_revert_commit = get_commit(store, &to_revert_oid)?;

    // Mainline verification for merge commits
    let parent_oid =
        if to_revert_commit.parents.len() > 1 {
            let m = mainline.ok_or_else(|| {
                anyhow::anyhow!(
                    "commit {} is a merge but no -m option was given.",
                    to_revert_oid.to_hex()
                )
            })?;
            if m == 0 || m > to_revert_commit.parents.len() {
                bail!(
                    "fatal: commit {} does not have parent #{}",
                    to_revert_oid.to_hex(),
                    m
                );
            }
            to_revert_commit.parents[m - 1]
        } else {
            to_revert_commit.parents.first().copied().ok_or_else(|| {
                anyhow::anyhow!("fatal: cannot revert a root commit with no parent")
            })?
        };

    let parent_commit = get_commit(store, &parent_oid)?;
    let head_state = crate::resolve_head(spine_dir)?;
    let head_oid = head_state
        .head_commit_id
        .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?;
    let head_commit = get_commit(store, &head_oid)?;

    // 3-way merge to invert changes:
    // Base = target_commit
    // Ours = HEAD
    // Theirs = target_commit.parent
    let mut base_files = BTreeMap::new();
    flatten_tree(store, &to_revert_commit.root_tree, "", &mut base_files)?;

    let mut ours_files = BTreeMap::new();
    flatten_tree(store, &head_commit.root_tree, "", &mut ours_files)?;

    let mut theirs_files = BTreeMap::new();
    flatten_tree(store, &parent_commit.root_tree, "", &mut theirs_files)?;

    let resolutions = merge_3way_trees(store, &base_files, &ours_files, &theirs_files)?;

    let mut has_conflicts = false;
    let mut path_conflicts = Vec::new();

    for (path, res) in resolutions {
        match res {
            ResolvedPath::KeepOurs => {}
            ResolvedPath::TakeTheirs {
                mode,
                object_id,
                content,
            }
            | ResolvedPath::MergedClean {
                mode,
                object_id,
                content,
            } => {
                let abs = worktree_root.join(&path);
                if let Some(p) = abs.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::write(&abs, &content)?;
                index.add_or_update(IndexEntry {
                    path: path.clone(),
                    mode,
                    file_size: content.len() as u64,
                    mtime_secs: 0,
                    mtime_nanos: 0,
                    inode: 0,
                    object_id,
                    stage_status: StageStatus::Staged,
                });
            }
            ResolvedPath::Delete => {
                let abs = worktree_root.join(&path);
                if abs.exists() {
                    let _ = fs::remove_file(&abs);
                }
                index.remove(&path);
            }
            ResolvedPath::Conflict(pc) => {
                has_conflicts = true;
                path_conflicts.push(pc);
                eprintln!("CONFLICT: Merge conflict in {}", path);
            }
        }
    }

    index.save_to_file(spine_dir.join("index"))?;

    let state = RevertState {
        target_commit: to_revert_oid,
        orig_head: head_oid,
        mainline,
    };

    if has_conflicts {
        state.save_to_dir(spine_dir)?;
        let ms = MergeState {
            source_branch: format!("revert-{}", &to_revert_oid.to_hex()[..7]),
            target_branch: head_state.head_ref_name.unwrap_or_else(|| "HEAD".into()),
            base_commit: to_revert_oid,
            ours_commit: head_oid,
            theirs_commit: parent_oid,
            conflicts: path_conflicts,
            resolved_paths: std::collections::BTreeSet::new(),
            timestamp_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };
        ms.save_to_dir(spine_dir)?;
        bail!("error: could not cleanly apply revert; resolve conflicts and run 'spine revert --continue'");
    }

    // Clean revert: create commit directly
    let new_tree_id = write_index_to_tree(index, store)?;
    let revert_msg = format!(
        "Revert \"{}\"\n\nThis reverts commit {}.",
        to_revert_commit.message.lines().next().unwrap_or(""),
        to_revert_oid.to_hex()
    );

    let new_commit = Commit::new_simple(
        new_tree_id,
        vec![head_oid],
        user_name.to_string(),
        user_email.to_string(),
        Utc::now().timestamp_nanos_opt().unwrap_or(0),
        revert_msg,
    );
    let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;

    if let Some(ref branch) = head_state.head_ref_name {
        crate::update_ref(
            primary_spine_dir,
            &format!("refs/heads/{}", branch),
            &new_cid,
        )?;
    } else {
        fs::write(spine_dir.join("HEAD"), format!("{}\n", new_cid.to_hex()))?;
    }

    println!(
        "[{}] Reverted commit {}",
        &new_cid.to_hex()[..7],
        &to_revert_oid.to_hex()[..7]
    );
    Ok(())
}

pub fn cmd_cherry_pick(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    target_oid: Option<ObjectId>,
    mainline: Option<usize>,
    continue_cp: bool,
    skip_cp: bool,
    abort_cp: bool,
) -> Result<()> {
    if abort_cp {
        let state = CherryPickState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no cherry-pick in progress to abort"))?;

        let orig_commit = get_commit(store, &state.orig_head)?;
        execute_switch(worktree_root, index, store, None, &orig_commit.root_tree)?;
        index.save_to_file(spine_dir.join("index"))?;

        let current_state = crate::resolve_head(spine_dir)?;
        if let Some(ref branch) = current_state.head_ref_name {
            crate::update_ref(
                primary_spine_dir,
                &format!("refs/heads/{}", branch),
                &state.orig_head,
            )?;
        } else {
            fs::write(
                spine_dir.join("HEAD"),
                format!("{}\n", state.orig_head.to_hex()),
            )?;
        }

        CherryPickState::clear(spine_dir)?;
        println!(
            "Cherry-pick aborted: restored HEAD to {}",
            &state.orig_head.to_hex()[..7]
        );
        return Ok(());
    }

    if skip_cp {
        CherryPickState::clear(spine_dir)?;
        println!("Skipped current cherry-pick commit");
        return Ok(());
    }

    if continue_cp {
        let state = CherryPickState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no cherry-pick in progress to continue"))?;

        if let Some(ms) = MergeState::load_from_dir(spine_dir)? {
            if !ms.is_fully_resolved() {
                bail!("fatal: Exiting because of unresolved conflicts. Stage resolved files with 'spine add' before continuing.");
            }
            MergeState::clear(spine_dir)?;
        }

        let new_tree_id = write_index_to_tree(index, store)?;
        let target_commit = get_commit(store, &state.target_commit)?;

        let head_state = crate::resolve_head(spine_dir)?;
        let head_oid = head_state.head_commit_id.unwrap();

        let new_commit = Commit::new_simple(
            new_tree_id,
            vec![head_oid],
            target_commit.author_name,
            target_commit.author_email,
            target_commit.timestamp_nanos,
            target_commit.message,
        );
        let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;

        if let Some(ref branch) = head_state.head_ref_name {
            crate::update_ref(
                primary_spine_dir,
                &format!("refs/heads/{}", branch),
                &new_cid,
            )?;
        } else {
            fs::write(spine_dir.join("HEAD"), format!("{}\n", new_cid.to_hex()))?;
        }

        CherryPickState::clear(spine_dir)?;
        println!(
            "[{}] Cherry-picked commit {}",
            &new_cid.to_hex()[..7],
            &state.target_commit.to_hex()[..7]
        );
        return Ok(());
    }

    let cp_oid = target_oid
        .ok_or_else(|| anyhow::anyhow!("fatal: commit revision required for cherry-pick"))?;
    let cp_commit = get_commit(store, &cp_oid)?;

    let parent_oid = if cp_commit.parents.len() > 1 {
        let m = mainline.ok_or_else(|| {
            anyhow::anyhow!(
                "commit {} is a merge but no -m option was given.",
                cp_oid.to_hex()
            )
        })?;
        if m == 0 || m > cp_commit.parents.len() {
            bail!(
                "fatal: commit {} does not have parent #{}",
                cp_oid.to_hex(),
                m
            );
        }
        cp_commit.parents[m - 1]
    } else {
        cp_commit.parents.first().copied().unwrap_or(ObjectId::ZERO)
    };

    let head_state = crate::resolve_head(spine_dir)?;
    let head_oid = head_state
        .head_commit_id
        .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?;
    let head_commit = get_commit(store, &head_oid)?;

    // 3-way merge:
    // Base = cp_commit.parent
    // Ours = HEAD
    // Theirs = cp_commit
    let mut base_files = BTreeMap::new();
    if parent_oid != ObjectId::ZERO {
        let parent_commit = get_commit(store, &parent_oid)?;
        flatten_tree(store, &parent_commit.root_tree, "", &mut base_files)?;
    }

    let mut ours_files = BTreeMap::new();
    flatten_tree(store, &head_commit.root_tree, "", &mut ours_files)?;

    let mut theirs_files = BTreeMap::new();
    flatten_tree(store, &cp_commit.root_tree, "", &mut theirs_files)?;

    let resolutions = merge_3way_trees(store, &base_files, &ours_files, &theirs_files)?;

    let mut has_conflicts = false;
    let mut path_conflicts = Vec::new();

    for (path, res) in resolutions {
        match res {
            ResolvedPath::KeepOurs => {}
            ResolvedPath::TakeTheirs {
                mode,
                object_id,
                content,
            }
            | ResolvedPath::MergedClean {
                mode,
                object_id,
                content,
            } => {
                let abs = worktree_root.join(&path);
                if let Some(p) = abs.parent() {
                    fs::create_dir_all(p)?;
                }
                fs::write(&abs, &content)?;
                index.add_or_update(IndexEntry {
                    path: path.clone(),
                    mode,
                    file_size: content.len() as u64,
                    mtime_secs: 0,
                    mtime_nanos: 0,
                    inode: 0,
                    object_id,
                    stage_status: StageStatus::Staged,
                });
            }
            ResolvedPath::Delete => {
                let abs = worktree_root.join(&path);
                if abs.exists() {
                    let _ = fs::remove_file(&abs);
                }
                index.remove(&path);
            }
            ResolvedPath::Conflict(pc) => {
                has_conflicts = true;
                path_conflicts.push(pc);
                eprintln!("CONFLICT: Merge conflict in {}", path);
            }
        }
    }

    index.save_to_file(spine_dir.join("index"))?;

    let state = CherryPickState {
        target_commit: cp_oid,
        orig_head: head_oid,
        mainline,
    };

    if has_conflicts {
        state.save_to_dir(spine_dir)?;
        let ms = MergeState {
            source_branch: format!("cherry-pick-{}", &cp_oid.to_hex()[..7]),
            target_branch: head_state.head_ref_name.unwrap_or_else(|| "HEAD".into()),
            base_commit: parent_oid,
            ours_commit: head_oid,
            theirs_commit: cp_oid,
            conflicts: path_conflicts,
            resolved_paths: std::collections::BTreeSet::new(),
            timestamp_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };
        ms.save_to_dir(spine_dir)?;
        bail!("error: could not cleanly apply cherry-pick; resolve conflicts and run 'spine cherry-pick --continue'");
    }

    let new_tree_id = write_index_to_tree(index, store)?;
    let new_commit = Commit::new_simple(
        new_tree_id,
        vec![head_oid],
        cp_commit.author_name,
        cp_commit.author_email,
        cp_commit.timestamp_nanos,
        cp_commit.message,
    );
    let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;

    if let Some(ref branch) = head_state.head_ref_name {
        crate::update_ref(
            primary_spine_dir,
            &format!("refs/heads/{}", branch),
            &new_cid,
        )?;
    } else {
        fs::write(spine_dir.join("HEAD"), format!("{}\n", new_cid.to_hex()))?;
    }

    println!(
        "[{}] Cherry-picked commit {}",
        &new_cid.to_hex()[..7],
        &cp_oid.to_hex()[..7]
    );
    Ok(())
}

pub fn cmd_rebase(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    upstream_oid: Option<ObjectId>,
    onto_oid: Option<ObjectId>,
    continue_rebase: bool,
    skip_rebase: bool,
    abort_rebase: bool,
) -> Result<()> {
    if abort_rebase {
        let state = RebaseState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no rebase in progress to abort"))?;

        let orig_commit = get_commit(store, &state.orig_head)?;
        execute_switch(worktree_root, index, store, None, &orig_commit.root_tree)?;
        index.save_to_file(spine_dir.join("index"))?;

        if let Some(ref branch) = state.branch_name {
            crate::update_ref(
                primary_spine_dir,
                &format!("refs/heads/{}", branch),
                &state.orig_head,
            )?;
            fs::write(
                spine_dir.join("HEAD"),
                format!("ref: refs/heads/{}\n", branch),
            )?;
        } else {
            fs::write(
                spine_dir.join("HEAD"),
                format!("{}\n", state.orig_head.to_hex()),
            )?;
        }

        RebaseState::clear(spine_dir)?;
        println!(
            "Rebase aborted: restored HEAD to {}",
            &state.orig_head.to_hex()[..7]
        );
        return Ok(());
    }

    if skip_rebase {
        let mut state = RebaseState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no rebase in progress to skip"))?;

        state.current_commit = None;
        return replay_remaining_rebase(
            worktree_root,
            spine_dir,
            primary_spine_dir,
            index,
            store,
            &mut state,
        );
    }

    if continue_rebase {
        let mut state = RebaseState::load_from_dir(spine_dir)?
            .ok_or_else(|| anyhow::anyhow!("fatal: no rebase in progress to continue"))?;

        if let Some(ms) = MergeState::load_from_dir(spine_dir)? {
            if !ms.is_fully_resolved() {
                bail!("fatal: Exiting because of unresolved conflicts. Stage resolved files with 'spine add' before continuing.");
            }
            MergeState::clear(spine_dir)?;
        }

        if let Some(curr) = state.current_commit {
            let new_tree_id = write_index_to_tree(index, store)?;
            let curr_commit = get_commit(store, &curr)?;

            let new_commit = Commit::new_simple(
                new_tree_id,
                vec![state.onto],
                curr_commit.author_name,
                curr_commit.author_email,
                curr_commit.timestamp_nanos,
                curr_commit.message,
            );
            let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;
            state.old_to_new.push((curr, new_cid));
            state.onto = new_cid;
            state.current_commit = None;
        }

        return replay_remaining_rebase(
            worktree_root,
            spine_dir,
            primary_spine_dir,
            index,
            store,
            &mut state,
        );
    }

    // Starting new rebase
    let upstream =
        upstream_oid.ok_or_else(|| anyhow::anyhow!("fatal: upstream required for rebase"))?;
    let onto = onto_oid.unwrap_or(upstream);

    let head_state = crate::resolve_head(spine_dir)?;
    let head_oid = head_state
        .head_commit_id
        .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?;

    // Find merge base
    let mb_opt = spine_history::find_unique_merge_base(store, &upstream, &head_oid)?;
    let mb = mb_opt.unwrap_or(ObjectId::ZERO);

    if mb == head_oid {
        println!("Current branch is up to date.");
        return Ok(());
    }

    let commits_to_replay = find_commits_between(store, &mb, &head_oid)?;
    if commits_to_replay.is_empty() {
        println!("No commits to replay.");
        return Ok(());
    }

    let head_commit = get_commit(store, &head_oid)?;
    let onto_commit = get_commit(store, &onto)?;
    execute_switch(
        worktree_root,
        index,
        store,
        Some(&head_commit.root_tree),
        &onto_commit.root_tree,
    )?;
    index.save_to_file(spine_dir.join("index"))?;

    let mut state = RebaseState {
        onto,
        orig_head: head_oid,
        branch_name: head_state.head_ref_name.clone(),
        remaining_commits: commits_to_replay,
        current_commit: None,
        old_to_new: Vec::new(),
    };

    replay_remaining_rebase(
        worktree_root,
        spine_dir,
        primary_spine_dir,
        index,
        store,
        &mut state,
    )
}

fn replay_remaining_rebase(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    state: &mut RebaseState,
) -> Result<()> {
    while !state.remaining_commits.is_empty() {
        let commit_id = state.remaining_commits.remove(0);
        state.current_commit = Some(commit_id);

        let commit = get_commit(store, &commit_id)?;
        let parent_oid = commit.parents.first().copied().unwrap_or(ObjectId::ZERO);

        let mut base_files = BTreeMap::new();
        if parent_oid != ObjectId::ZERO {
            let p_commit = get_commit(store, &parent_oid)?;
            flatten_tree(store, &p_commit.root_tree, "", &mut base_files)?;
        }

        let onto_commit = get_commit(store, &state.onto)?;
        let mut ours_files = BTreeMap::new();
        flatten_tree(store, &onto_commit.root_tree, "", &mut ours_files)?;

        let mut theirs_files = BTreeMap::new();
        flatten_tree(store, &commit.root_tree, "", &mut theirs_files)?;

        let resolutions = merge_3way_trees(store, &base_files, &ours_files, &theirs_files)?;
        let mut has_conflicts = false;
        let mut path_conflicts = Vec::new();

        for (path, res) in resolutions {
            match res {
                ResolvedPath::KeepOurs => {}
                ResolvedPath::TakeTheirs {
                    mode,
                    object_id,
                    content,
                }
                | ResolvedPath::MergedClean {
                    mode,
                    object_id,
                    content,
                } => {
                    let abs = worktree_root.join(&path);
                    if let Some(p) = abs.parent() {
                        fs::create_dir_all(p)?;
                    }
                    fs::write(&abs, &content)?;
                    index.add_or_update(IndexEntry {
                        path: path.clone(),
                        mode,
                        file_size: content.len() as u64,
                        mtime_secs: 0,
                        mtime_nanos: 0,
                        inode: 0,
                        object_id,
                        stage_status: StageStatus::Staged,
                    });
                }
                ResolvedPath::Delete => {
                    let abs = worktree_root.join(&path);
                    if abs.exists() {
                        let _ = fs::remove_file(&abs);
                    }
                    index.remove(&path);
                }
                ResolvedPath::Conflict(pc) => {
                    has_conflicts = true;
                    path_conflicts.push(pc);
                    eprintln!("CONFLICT: Merge conflict in {}", path);
                }
            }
        }

        index.save_to_file(spine_dir.join("index"))?;

        if has_conflicts {
            state.save_to_dir(spine_dir)?;
            let ms = MergeState {
                source_branch: format!("rebase-{}", &commit_id.to_hex()[..7]),
                target_branch: state.branch_name.clone().unwrap_or_else(|| "HEAD".into()),
                base_commit: parent_oid,
                ours_commit: state.onto,
                theirs_commit: commit_id,
                conflicts: path_conflicts,
                resolved_paths: std::collections::BTreeSet::new(),
                timestamp_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
            };
            ms.save_to_dir(spine_dir)?;
            bail!(
                "error: could not cleanly apply {}; resolve conflicts and run 'spine rebase --continue'",
                &commit_id.to_hex()[..7]
            );
        }

        // Clean apply
        let new_tree_id = write_index_to_tree(index, store)?;
        let new_commit = Commit::new_simple(
            new_tree_id,
            vec![state.onto],
            commit.author_name,
            commit.author_email,
            commit.timestamp_nanos,
            commit.message,
        );
        let new_cid = store.write_object(ObjectType::Commit, &serialize_commit(&new_commit))?;
        state.old_to_new.push((commit_id, new_cid));
        state.onto = new_cid;
        state.current_commit = None;
    }

    // All commits replayed successfully
    if let Some(ref branch) = state.branch_name {
        crate::update_ref(
            primary_spine_dir,
            &format!("refs/heads/{}", branch),
            &state.onto,
        )?;
        fs::write(
            spine_dir.join("HEAD"),
            format!("ref: refs/heads/{}\n", branch),
        )?;
    } else {
        fs::write(spine_dir.join("HEAD"), format!("{}\n", state.onto.to_hex()))?;
    }

    RebaseState::clear(spine_dir)?;
    println!(
        "Successfully rebased and updated branch tip to {}",
        &state.onto.to_hex()[..7]
    );
    Ok(())
}

pub fn cmd_blame(store: &CompositeObjectStore, target_oid: &ObjectId, path: &str) -> Result<()> {
    let blame = compute_blame(store, target_oid, path)?;

    for line in blame.lines {
        let date_str = if let Some(dt) = Utc
            .timestamp_opt(line.timestamp_nanos / 1_000_000_000, 0)
            .single()
        {
            dt.format("%Y-%m-%d %H:%M").to_string()
        } else {
            "----".to_string()
        };

        println!(
            "{} ({:<15} {} {:>4}) {}",
            &line.commit_id.to_hex()[..8],
            line.author_name,
            date_str,
            line.line_number,
            line.content
        );
    }
    Ok(())
}

pub fn cmd_bisect(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    subcmd: &crate::BisectAction,
) -> Result<()> {
    use crate::BisectAction;

    match subcmd {
        BisectAction::Start { bad, good } => {
            let head_state = crate::resolve_head(spine_dir)?;
            let head_oid = head_state
                .head_commit_id
                .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?;

            let bad_oid = match bad {
                Some(b) => {
                    crate::resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, b)?
                }
                None => head_oid,
            };

            let mut good_commits = Vec::new();
            if let Some(g) = good {
                let g_oid =
                    crate::resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, g)?;
                good_commits.push(g_oid);
            }

            let mut state =
                BisectState::new(bad_oid, good_commits, head_oid, head_state.head_ref_name);

            step_bisect(worktree_root, spine_dir, index, store, &mut state)?;
            state.save_to_dir(spine_dir)?;
        }
        BisectAction::Bad { rev } => {
            let mut state = BisectState::load_from_dir(spine_dir)?
                .ok_or_else(|| anyhow::anyhow!("fatal: no bisect session in progress"))?;

            let b_oid = match rev {
                Some(r) => {
                    crate::resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, r)?
                }
                None => crate::resolve_head(spine_dir)?.head_commit_id.unwrap(),
            };
            state.bad_commit = b_oid;

            step_bisect(worktree_root, spine_dir, index, store, &mut state)?;
            state.save_to_dir(spine_dir)?;
        }
        BisectAction::Good { rev } => {
            let mut state = BisectState::load_from_dir(spine_dir)?
                .ok_or_else(|| anyhow::anyhow!("fatal: no bisect session in progress"))?;

            let g_oid = match rev {
                Some(r) => {
                    crate::resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, r)?
                }
                None => crate::resolve_head(spine_dir)?.head_commit_id.unwrap(),
            };
            if !state.good_commits.contains(&g_oid) {
                state.good_commits.push(g_oid);
            }

            step_bisect(worktree_root, spine_dir, index, store, &mut state)?;
            state.save_to_dir(spine_dir)?;
        }
        BisectAction::Skip { rev } => {
            let mut state = BisectState::load_from_dir(spine_dir)?
                .ok_or_else(|| anyhow::anyhow!("fatal: no bisect session in progress"))?;

            let s_oid = match rev {
                Some(r) => {
                    crate::resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, r)?
                }
                None => crate::resolve_head(spine_dir)?.head_commit_id.unwrap(),
            };
            if !state.skipped_commits.contains(&s_oid) {
                state.skipped_commits.push(s_oid);
            }

            step_bisect(worktree_root, spine_dir, index, store, &mut state)?;
            state.save_to_dir(spine_dir)?;
        }
        BisectAction::Status => {
            let state = BisectState::load_from_dir(spine_dir)?
                .ok_or_else(|| anyhow::anyhow!("fatal: no bisect session in progress"))?;

            println!("Bisect status:");
            println!("  Bad commit:     {}", state.bad_commit.to_hex());
            for (i, g) in state.good_commits.iter().enumerate() {
                println!("  Good commit #{}: {}", i + 1, g.to_hex());
            }
            if !state.skipped_commits.is_empty() {
                println!("  Skipped:        {} commits", state.skipped_commits.len());
            }
            if let Some(curr) = state.current_commit {
                println!("  Currently testing: {}", curr.to_hex());
            }
        }
        BisectAction::Reset => {
            let state = BisectState::load_from_dir(spine_dir)?
                .ok_or_else(|| anyhow::anyhow!("fatal: no bisect session in progress"))?;

            let orig_commit = get_commit(store, &state.orig_head)?;
            execute_switch(worktree_root, index, store, None, &orig_commit.root_tree)?;
            index.save_to_file(spine_dir.join("index"))?;

            if let Some(ref branch) = state.orig_branch {
                fs::write(
                    spine_dir.join("HEAD"),
                    format!("ref: refs/heads/{}\n", branch),
                )?;
            } else {
                fs::write(
                    spine_dir.join("HEAD"),
                    format!("{}\n", state.orig_head.to_hex()),
                )?;
            }

            BisectState::clear(spine_dir)?;
            println!(
                "Bisect reset: checkout back to orig HEAD ({})",
                &state.orig_head.to_hex()[..7]
            );
        }
    }

    Ok(())
}

fn step_bisect(
    worktree_root: &Path,
    spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    state: &mut BisectState,
) -> Result<()> {
    if state.good_commits.is_empty() {
        println!("Bisecting: waiting for both good and bad commits");
        return Ok(());
    }

    let skipped_set: HashSet<ObjectId> = state.skipped_commits.iter().copied().collect();
    let midpoint_opt =
        find_bisect_midpoint(store, &state.good_commits, &state.bad_commit, &skipped_set)?;

    match midpoint_opt {
        Some(mid) => {
            if mid == state.bad_commit {
                println!("{} is the first bad commit", state.bad_commit.to_hex());
                let c = get_commit(store, &state.bad_commit)?;
                println!(
                    "Author: {}\nDate:   {}\n\n    {}",
                    c.author_name, c.timestamp_nanos, c.message
                );
                return Ok(());
            }

            state.current_commit = Some(mid);
            let mid_commit = get_commit(store, &mid)?;
            execute_switch(worktree_root, index, store, None, &mid_commit.root_tree)?;
            index.save_to_file(spine_dir.join("index"))?;
            fs::write(spine_dir.join("HEAD"), format!("{}\n", mid.to_hex()))?;

            println!(
                "Bisecting: checking out candidate {} (detached HEAD)",
                &mid.to_hex()[..7]
            );
        }
        None => {
            println!("{} is the first bad commit", state.bad_commit.to_hex());
            let c = get_commit(store, &state.bad_commit)?;
            println!(
                "Author: {}\nDate:   {}\n\n    {}",
                c.author_name, c.timestamp_nanos, c.message
            );
        }
    }

    Ok(())
}
