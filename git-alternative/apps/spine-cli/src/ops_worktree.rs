use anyhow::{bail, Context, Result};
use spine_core::ObjectId;
use spine_diff::{compare_trees, format_diff_stat};
use spine_index::{Index, IndexEntry, StageStatus};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use spine_worktree::{
    add_worktree, compute_status, flatten_tree, list_worktrees, normalize_rel_path,
    prune_worktrees, remove_worktree, stash_apply, stash_clear, stash_drop, stash_pop, stash_push,
    IgnoreRules, StashList,
};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[allow(clippy::too_many_arguments)]
pub fn cmd_stash_push(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    head_oid: Option<&ObjectId>,
    branch_name: &str,
    message: Option<&str>,
    include_untracked: bool,
    ignore: &IgnoreRules,
) -> Result<()> {
    let entry = stash_push(
        worktree_root,
        primary_spine_dir,
        index,
        store,
        head_oid,
        branch_name,
        message,
        include_untracked,
        ignore,
    )?;

    index.save_to_file(spine_dir.join("index"))?;

    match entry {
        Some(e) => {
            println!("Saved working directory and index state {}", e.message);
        }
        None => {
            println!("No local changes to save");
        }
    }
    Ok(())
}

pub fn cmd_stash_list(primary_spine_dir: &Path) -> Result<()> {
    let list = StashList::load_from_dir(primary_spine_dir)?;
    if list.entries.is_empty() {
        return Ok(());
    }
    for entry in &list.entries {
        println!("stash@{{{}}}: {}", entry.index, entry.message);
    }
    Ok(())
}

pub fn cmd_stash_show(
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    index_num: usize,
) -> Result<()> {
    let list = StashList::load_from_dir(primary_spine_dir)?;
    let entry = list.get(index_num).ok_or_else(|| {
        anyhow::anyhow!("fatal: 'stash@{{{}}}' is not a stash-like regex", index_num)
    })?;

    let mut base_files = BTreeMap::new();
    let (_, commit_bytes) = store.read_object(&entry.commit_id)?;
    let commit = spine_format::deserialize_commit(&commit_bytes)?;
    if let Some(parent) = commit.parents.first() {
        let (_, p_bytes) = store.read_object(parent)?;
        let p_commit = spine_format::deserialize_commit(&p_bytes)?;
        flatten_tree(store, &p_commit.root_tree, "", &mut base_files)?;
    }

    let mut stash_files = BTreeMap::new();
    flatten_tree(store, &entry.worktree_tree, "", &mut stash_files)?;

    let changes = compare_trees(&base_files, &stash_files);
    let summary = crate::summarize_diff(&changes, store);
    print!("{}", format_diff_stat(&summary));
    Ok(())
}

pub fn cmd_stash_apply(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    index_num: usize,
    reinstate_index: bool,
) -> Result<()> {
    let entry = stash_apply(
        worktree_root,
        primary_spine_dir,
        index,
        store,
        index_num,
        reinstate_index,
    )?;
    index.save_to_file(spine_dir.join("index"))?;
    println!("Applied stash@{{{}}} ({})", index_num, entry.message);
    Ok(())
}

pub fn cmd_stash_pop(
    worktree_root: &Path,
    spine_dir: &Path,
    primary_spine_dir: &Path,
    index: &mut Index,
    store: &CompositeObjectStore,
    index_num: usize,
    reinstate_index: bool,
) -> Result<()> {
    let entry = stash_pop(
        worktree_root,
        primary_spine_dir,
        index,
        store,
        index_num,
        reinstate_index,
    )?;
    index.save_to_file(spine_dir.join("index"))?;
    println!(
        "Dropped and applied stash@{{{}}} ({})",
        index_num, entry.message
    );
    Ok(())
}

pub fn cmd_stash_drop(primary_spine_dir: &Path, index_num: usize) -> Result<()> {
    let entry = stash_drop(primary_spine_dir, index_num)?;
    println!(
        "Dropped stash@{{{}}} ({})",
        index_num,
        entry.commit_id.to_hex()
    );
    Ok(())
}

pub fn cmd_stash_clear(primary_spine_dir: &Path) -> Result<()> {
    stash_clear(primary_spine_dir)?;
    println!("Cleared all stash entries");
    Ok(())
}

pub fn cmd_worktree_add(
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    path: &Path,
    branch: Option<String>,
    target_oid: &ObjectId,
) -> Result<()> {
    let wt = add_worktree(
        primary_spine_dir,
        path,
        branch.as_deref(),
        target_oid,
        store,
    )?;
    println!(
        "Preparing worktree (checking out '{}')",
        wt.branch.as_deref().unwrap_or(&target_oid.to_hex()[..7])
    );
    println!(
        "HEAD is now at {} {}",
        &target_oid.to_hex()[..7],
        wt.path.display()
    );
    Ok(())
}

pub fn cmd_worktree_list(primary_spine_dir: &Path) -> Result<()> {
    let worktrees = list_worktrees(primary_spine_dir)?;
    for wt in worktrees {
        let branch_display = wt
            .branch
            .map(|b| format!("[{}]", b))
            .unwrap_or_else(|| "(detached HEAD)".to_string());
        let hash_display = wt
            .head_oid
            .map(|h| h.to_hex()[..7].to_string())
            .unwrap_or_else(|| "0000000".to_string());
        let prunable = if wt.is_prunable { " (prunable)" } else { "" };
        let locked = if wt.locked { " (locked)" } else { "" };

        println!(
            "{:<40} {:<8} {}{}{}",
            wt.path.display(),
            hash_display,
            branch_display,
            prunable,
            locked
        );
    }
    Ok(())
}

pub fn cmd_worktree_remove(primary_spine_dir: &Path, id_or_path: &str, force: bool) -> Result<()> {
    let removed_path = remove_worktree(primary_spine_dir, id_or_path, force)?;
    println!("Removed worktree '{}'", removed_path.display());
    Ok(())
}

pub fn cmd_worktree_prune(primary_spine_dir: &Path) -> Result<()> {
    let pruned = prune_worktrees(primary_spine_dir)?;
    if pruned.is_empty() {
        println!("No worktrees to prune");
    } else {
        for id in pruned {
            println!("Pruned worktree '{}'", id);
        }
    }
    Ok(())
}

pub fn cmd_mv(
    worktree_root: &Path,
    spine_dir: &Path,
    index: &mut Index,
    source: &Path,
    dest: &Path,
) -> Result<()> {
    let abs_src = if source.is_absolute() {
        source.to_path_buf()
    } else {
        worktree_root.join(source)
    };
    let abs_dst = if dest.is_absolute() {
        dest.to_path_buf()
    } else {
        worktree_root.join(dest)
    };

    if !abs_src.exists() {
        bail!("fatal: source '{}' does not exist", source.display());
    }

    let rel_src = abs_src
        .strip_prefix(worktree_root)
        .context("Source path must be inside repository")?;
    let norm_src = normalize_rel_path(rel_src)?;

    let src_entry = index
        .get(&norm_src)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("fatal: not a tracked file, cannot move: {}", norm_src))?;

    // Determine final destination path (if dest is an existing directory)
    let final_dst = if abs_dst.is_dir() {
        abs_dst.join(abs_src.file_name().unwrap())
    } else {
        abs_dst
    };

    if final_dst.exists() && abs_src != final_dst {
        bail!(
            "fatal: destination '{}' already exists",
            final_dst.display()
        );
    }

    if let Some(parent) = final_dst.parent() {
        fs::create_dir_all(parent)?;
    }

    // Move on disk
    fs::rename(&abs_src, &final_dst)?;

    // Move in index
    let rel_dst = final_dst
        .strip_prefix(worktree_root)
        .context("Destination path must be inside repository")?;
    let norm_dst = normalize_rel_path(rel_dst)?;

    index.remove(&norm_src);
    let meta = fs::metadata(&final_dst)?;
    let (mode, size, mtime_secs, mtime_nanos) = spine_worktree::metadata_to_stat(&meta);

    index.add_or_update(IndexEntry {
        path: norm_dst.clone(),
        mode,
        file_size: size,
        mtime_secs,
        mtime_nanos,
        inode: 0,
        object_id: src_entry.object_id,
        stage_status: StageStatus::Staged,
    });

    index.save_to_file(spine_dir.join("index"))?;
    println!("Renamed '{}' -> '{}'", norm_src, norm_dst);
    Ok(())
}

pub fn cmd_rm(
    worktree_root: &Path,
    spine_dir: &Path,
    index: &mut Index,
    paths: &[PathBuf],
    cached: bool,
    force: bool,
) -> Result<()> {
    let mut removed_count = 0;

    for target in paths {
        let abs_target = if target.is_absolute() {
            target.clone()
        } else {
            worktree_root.join(target)
        };

        let rel = match abs_target.strip_prefix(worktree_root) {
            Ok(r) => r,
            Err(_) => bail!("fatal: path '{}' is outside repository", target.display()),
        };
        let norm = normalize_rel_path(rel)?;

        // Find all matching entries in index (support directory prefix or single file)
        let matching: Vec<String> = index
            .iter()
            .filter(|e| e.path == norm || e.path.starts_with(&format!("{}/", norm)))
            .map(|e| e.path.clone())
            .collect();

        if matching.is_empty() {
            bail!("fatal: pathspec '{}' did not match any files", norm);
        }

        for path in matching {
            let file_abs = worktree_root.join(&path);
            if !cached && file_abs.exists() {
                // Check if dirty unless force
                if !force {
                    let _disk_content = fs::read(&file_abs)?;
                    let (_mode, size, mtime_secs, mtime_nanos) =
                        spine_worktree::metadata_to_stat(&fs::metadata(&file_abs)?);
                    let cur_entry = index.get(&path).unwrap();
                    if size != cur_entry.file_size
                        || mtime_secs != cur_entry.mtime_secs
                        || mtime_nanos != cur_entry.mtime_nanos
                    {
                        bail!(
                            "error: the following file has local modifications:\n  {}\n(use --force to discard local changes)",
                            path
                        );
                    }
                }
                fs::remove_file(&file_abs)?;
            }
            index.remove(&path);
            println!("rm '{}'", path);
            removed_count += 1;
        }
    }

    if removed_count > 0 {
        index.save_to_file(spine_dir.join("index"))?;
    }
    Ok(())
}

pub fn cmd_clean(
    worktree_root: &Path,
    index: &Index,
    store: &CompositeObjectStore,
    head_tree_id: Option<&ObjectId>,
    dry_run: bool,
    force: bool,
    ignore: &IgnoreRules,
) -> Result<()> {
    if !dry_run && !force {
        bail!("fatal: clean requires -f/--force to prevent accidental data loss");
    }

    let status = compute_status(worktree_root, index, store, head_tree_id, ignore)?;
    if status.untracked.is_empty() {
        return Ok(());
    }

    for untracked in &status.untracked {
        let abs = worktree_root.join(untracked);
        if dry_run {
            println!("Would remove {}", untracked);
        } else {
            if abs.is_dir() {
                let _ = fs::remove_dir_all(&abs);
            } else if abs.exists() {
                let _ = fs::remove_file(&abs);
            }
            println!("Removing {}", untracked);
        }
    }

    Ok(())
}
