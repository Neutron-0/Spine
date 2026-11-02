pub mod ops_history;
pub mod ops_tag;
pub mod ops_worktree;

use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use clap::{Parser, Subcommand};
use spine_config::SpineConfig;
use spine_core::{Commit, HashAlgorithm, ObjectId, ObjectType};
use spine_diff::{compare_trees, compute_line_diff, format_diff_stat, format_unified_diff};
use spine_format::serialize_commit;
use spine_history::{find_best_merge_base, get_commit, is_ancestor, walk_history};
use spine_index::{Index, IndexEntry, StageStatus};
use spine_integrity::run_fsck;
use spine_journal::{Journal, OperationKind, RepoStateSnapshot};
use spine_merge::{merge_3way_text, merge_3way_trees, MergeState, ResolvedPath, TextMergeResult};
use spine_net::{SpineClient, SpineUrl};
use spine_object_store::{
    collect_garbage, repack_loose_objects, CompositeObjectStore, ObjectStore, PackIndexReader,
    PackReader, RepoLock, DEFAULT_GRACE_PERIOD_SECS,
};
use spine_protocol::messages::RefUpdateProposal;
use spine_worktree::{
    check_switch_safety, compute_status, execute_switch, flatten_tree, metadata_to_stat,
    stage_path, write_index_to_tree, IgnoreRules,
};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use uuid::Uuid;

#[derive(Parser)]
#[command(
    name = "spine",
    about = "Spine — High-performance Rust-native Version Control System",
    version = "0.2.0"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Initialize a new native Spine repository
    Init {
        /// Target directory (defaults to current directory)
        path: Option<PathBuf>,
    },
    /// Manage configuration parameters
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Stage files or directories for the next commit
    Add {
        /// Files, directories, or '.' to stage
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Show working tree and staging status
    Status,
    /// Record changes to the repository
    Commit {
        /// Commit message
        #[arg(short, long)]
        message: String,
    },
    /// Show commit history logs
    Log {
        /// Maximum number of commits to list
        #[arg(short, long)]
        limit: Option<usize>,
    },
    /// Show details of a commit or object
    Show {
        /// Commit hash or 'HEAD' (default)
        #[arg(default_value = "HEAD")]
        object: String,
    },
    /// Manage repository branches
    Branch {
        #[command(subcommand)]
        action: Option<BranchAction>,
    },
    /// Switch branches or restore working tree
    Switch {
        /// Branch name to switch to
        branch: String,
        /// Create a new branch and switch to it
        #[arg(short, long)]
        create: bool,
    },
    /// Show changes between commits, commit and working tree, etc
    Diff {
        /// Show staged changes against HEAD
        #[arg(long)]
        staged: bool,
        /// Show diffstat summary
        #[arg(long)]
        stat: bool,
        /// First revision or branch (optional)
        rev_a: Option<String>,
        /// Second revision or branch (optional)
        rev_b: Option<String>,
    },
    /// Join two or more development histories together
    Merge {
        /// Branch to merge into current HEAD
        branch: Option<String>,
        /// Abort the current in-progress merge
        #[arg(long)]
        abort: bool,
        /// Continue the current merge after resolving conflicts
        #[arg(long = "continue")]
        continue_merge: bool,
        /// Create a merge commit even when fast-forward is possible
        #[arg(long)]
        no_ff: bool,
        /// Custom commit message for merge
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Undo the most recent repository state mutation
    Undo {
        /// List journal transactions instead of undoing
        #[arg(long)]
        list: bool,
    },
    /// Redo a previously undone operation
    Redo,
    /// Verify repository integrity and detect corruption
    Fsck,
    /// Count unpacked and packed objects and disk footprint
    CountObjects,
    /// Manage packfiles and indexes
    Pack {
        #[command(subcommand)]
        action: PackAction,
    },
    /// Consolidate loose objects into packfiles
    Repack {
        /// Simulate repacking without writing new packs or deleting loose objects
        #[arg(long)]
        dry_run: bool,
        /// Audit and verify every object in the newly created pack
        #[arg(long)]
        verify: bool,
    },
    /// Cleanup unreachable expired objects
    Gc {
        /// Simulate garbage collection without deleting any objects
        #[arg(long)]
        dry_run: bool,
        /// Prune objects older than duration (e.g. 14d, 2h, 0s; default 14d)
        #[arg(long, default_value = "14d")]
        prune_before: String,
    },
    /// Verify integrity of all packfiles, indexes, and storage catalog
    VerifyStorage,
    /// Manage set of tracked remote repositories
    Remote {
        #[command(subcommand)]
        action: Option<RemoteAction>,
    },
    /// Clone a remote repository into a new directory
    Clone {
        /// Remote repository URL (e.g. spine://127.0.0.1:4434/repo)
        url: String,
        /// Target directory to clone into
        directory: Option<PathBuf>,
    },
    /// Download objects and refs from another repository
    Fetch {
        /// Remote repository name (default "origin")
        #[arg(default_value = "origin")]
        remote: String,
        /// Fetch all remotes
        #[arg(long)]
        all: bool,
    },
    /// Update remote refs along with associated objects
    Push {
        /// Remote repository name (default "origin")
        #[arg(default_value = "origin")]
        remote: String,
        /// Branch name to push
        branch: Option<String>,
        /// Force update remote references (bypass fast-forward check)
        #[arg(short, long)]
        force: bool,
        /// Require remote branch to match expected OID or tracking tip before updating
        #[arg(long, num_args = 0..=1, default_missing_value = "")]
        force_with_lease: Option<String>,
    },
    /// Fetch from and integrate with another repository or branch
    Pull {
        /// Remote repository name (default "origin")
        #[arg(default_value = "origin")]
        remote: String,
        /// Branch name to pull
        branch: Option<String>,
    },
    /// Create, list, or delete tags
    Tag {
        #[command(subcommand)]
        action: Option<TagAction>,
    },
    /// Reset current HEAD to the specified state or unstage paths
    Reset {
        /// Move HEAD to revision, leave index and working tree intact
        #[arg(long)]
        soft: bool,
        /// Move HEAD and reset index, leave working tree intact (default)
        #[arg(long)]
        mixed: bool,
        /// Move HEAD, reset index and discard all changes in working tree
        #[arg(long)]
        hard: bool,
        /// Target revision (defaults to HEAD)
        #[arg(default_value = "HEAD")]
        revision: String,
        /// Path-specific reset
        paths: Vec<PathBuf>,
    },
    /// Restore working tree files or staging index
    Restore {
        /// Restore staging index
        #[arg(long)]
        staged: bool,
        /// Restore working tree (default)
        #[arg(long)]
        worktree: bool,
        /// Source revision (defaults to HEAD for staged, index for worktree)
        #[arg(short, long)]
        source: Option<String>,
        /// Paths to restore
        #[arg(required = true)]
        paths: Vec<PathBuf>,
    },
    /// Revert an existing commit by creating an inverted diff commit
    Revert {
        /// Commit revision to revert
        revision: Option<String>,
        /// Mainline parent number for merge commits (1-based)
        #[arg(short, long)]
        mainline: Option<usize>,
        /// Continue revert after resolving conflicts
        #[arg(long = "continue")]
        continue_revert: bool,
        /// Abort revert and restore original HEAD
        #[arg(long)]
        abort: bool,
    },
    /// Apply changes introduced by an existing commit onto HEAD
    CherryPick {
        /// Commit revision to cherry-pick
        revision: Option<String>,
        /// Mainline parent number for merge commits (1-based)
        #[arg(short, long)]
        mainline: Option<usize>,
        /// Continue cherry-pick after resolving conflicts
        #[arg(long = "continue")]
        continue_cherry_pick: bool,
        /// Skip the current commit and continue
        #[arg(long)]
        skip: bool,
        /// Abort cherry-pick and restore original HEAD
        #[arg(long)]
        abort: bool,
    },
    /// Reapply commits on top of another base tip
    Rebase {
        /// Upstream branch or commit
        upstream: Option<String>,
        /// New base commit to replay onto (defaults to upstream)
        #[arg(long)]
        onto: Option<String>,
        /// Continue rebase after resolving conflicts
        #[arg(long = "continue")]
        continue_rebase: bool,
        /// Skip the current commit and continue
        #[arg(long)]
        skip: bool,
        /// Abort rebase and restore original branch/HEAD
        #[arg(long)]
        abort: bool,
    },
    /// Stash the changes in a dirty working directory away
    Stash {
        #[command(subcommand)]
        action: Option<StashAction>,
    },
    /// Manage multiple linked working trees
    Worktree {
        #[command(subcommand)]
        action: WorktreeAction,
    },
    /// Show what revision and author last modified each line of a file
    Blame {
        /// File path to blame
        path: String,
        /// Commit revision (defaults to HEAD)
        #[arg(short, long, default_value = "HEAD")]
        revision: String,
    },
    /// Use binary search to find the commit that introduced a bug
    Bisect {
        #[command(subcommand)]
        action: BisectAction,
    },
    /// Move or rename a file or directory
    Mv {
        /// Source path
        source: PathBuf,
        /// Destination path
        destination: PathBuf,
    },
    /// Remove files from the working tree and index
    Rm {
        /// Paths to remove
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// Only remove from index; keep working tree files
        #[arg(long)]
        cached: bool,
        /// Force removal even if files have unstaged modifications
        #[arg(short, long)]
        force: bool,
    },
    /// Remove untracked files from the working tree
    Clean {
        /// Dry-run: show what would be removed
        #[arg(short = 'n', long)]
        dry_run: bool,
        /// Force removal of untracked files
        #[arg(short, long)]
        force: bool,
    },
}

#[derive(Subcommand)]
pub enum StashAction {
    /// Save local changes to a new stash
    Push {
        /// Stash message
        #[arg(short, long)]
        message: Option<String>,
        /// Include untracked files
        #[arg(short = 'u', long)]
        include_untracked: bool,
    },
    /// List all stash entries
    List,
    /// Show stash diff summary
    Show {
        #[arg(default_value = "0")]
        index: usize,
    },
    /// Apply stash entry
    Apply {
        #[arg(default_value = "0")]
        index: usize,
        /// Reinstate index staging state
        #[arg(long)]
        index_reinstate: bool,
    },
    /// Apply stash entry and remove it from stash list
    Pop {
        #[arg(default_value = "0")]
        index: usize,
        /// Reinstate index staging state
        #[arg(long)]
        index_reinstate: bool,
    },
    /// Remove a single stash entry
    Drop {
        #[arg(default_value = "0")]
        index: usize,
    },
    /// Clear all stash entries
    Clear,
}

#[derive(Subcommand)]
pub enum WorktreeAction {
    /// Add a new linked worktree
    Add {
        /// Worktree path
        path: PathBuf,
        /// Branch name to check out
        branch: Option<String>,
    },
    /// List all linked worktrees
    List,
    /// Remove a linked worktree
    Remove {
        /// Worktree ID or path
        id_or_path: String,
        /// Force removal even if worktree has modifications
        #[arg(short, long)]
        force: bool,
    },
    /// Prune worktree records whose disk directories no longer exist
    Prune,
}

#[derive(Subcommand)]
pub enum BisectAction {
    /// Start bisect session with optional bad and good commits
    Start {
        bad: Option<String>,
        good: Option<String>,
    },
    /// Mark a revision as bad
    Bad { rev: Option<String> },
    /// Mark a revision as good
    Good { rev: Option<String> },
    /// Skip current or specified revision
    Skip { rev: Option<String> },
    /// Show bisect session status
    Status,
    /// Abort bisect and return to original HEAD
    Reset,
}

#[derive(Subcommand)]
pub enum RemoteAction {
    /// Add a new remote repository
    Add { name: String, url: String },
    /// List all configured remotes
    List,
    /// Remove a remote
    Remove { name: String },
    /// Get URL for a named remote
    GetUrl { name: String },
    /// Change the URL for a named remote
    SetUrl { name: String, url: String },
}

#[derive(Subcommand)]
pub enum TagAction {
    /// List all tags
    List,
    /// Create a new tag pointing to rev (default HEAD)
    Create {
        name: String,
        #[arg(default_value = "HEAD")]
        rev: String,
        /// Create an annotated tag object
        #[arg(short, long)]
        annotate: bool,
        /// Tag message
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Show details of an annotated or lightweight tag
    Show { name: String },
    /// Verify cryptographic integrity of tag and target object
    Verify { name: String },
    /// Delete a tag
    Delete { name: String },
}

#[derive(Subcommand)]
enum PackAction {
    /// List all active packfiles in catalog
    List,
    /// Inspect a packfile and show record statistics
    Inspect {
        /// Pack ID (e.g. pack-<uuid>)
        id: String,
    },
}

#[derive(Subcommand)]
enum BranchAction {
    /// List all branches (default)
    List,
    /// Create a new branch
    Create {
        name: String,
        #[arg(long)]
        from: Option<String>,
    },
    /// Delete a branch
    Delete {
        name: String,
        #[arg(short, long)]
        force: bool,
    },
    /// Rename a branch
    Rename { old_name: String, new_name: String },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Retrieve a configuration value
    Get { key: String },
    /// Set a configuration value
    Set { key: String, value: String },
}

pub struct RepoContext {
    pub worktree_root: PathBuf,
    pub spine_dir: PathBuf,
    pub primary_spine_dir: PathBuf,
    pub is_linked: bool,
}

pub fn get_repo_context() -> Result<RepoContext> {
    let current = env::current_dir().context("Failed to get current working directory")?;
    let (worktree_root, spine_dir, primary_spine_dir, is_linked) =
        spine_worktree::resolve_worktree_context(&current).map_err(|_| {
            anyhow::anyhow!(
                "fatal: not a spine repository (or any of the parent directories): .spine"
            )
        })?;
    Ok(RepoContext {
        worktree_root,
        spine_dir,
        primary_spine_dir,
        is_linked,
    })
}

pub fn find_repo_root() -> Result<(PathBuf, PathBuf)> {
    let ctx = get_repo_context()?;
    Ok((ctx.worktree_root, ctx.spine_dir))
}

pub fn load_repo_config(spine_dir: &Path) -> Result<SpineConfig> {
    let effective_spine_dir = if spine_dir
        .parent()
        .and_then(|p| p.file_name())
        .map_or(false, |n| n == "worktrees")
    {
        spine_dir.parent().unwrap().parent().unwrap()
    } else {
        spine_dir
    };
    let config_path = effective_spine_dir.join("spine.toml");
    if config_path.exists() {
        Ok(SpineConfig::load_from_file(&config_path)?)
    } else {
        Ok(SpineConfig::default())
    }
}

pub fn open_store(spine_dir: &Path, config: &SpineConfig) -> Result<CompositeObjectStore> {
    let effective_spine_dir = if spine_dir
        .parent()
        .and_then(|p| p.file_name())
        .map_or(false, |n| n == "worktrees")
    {
        spine_dir.parent().unwrap().parent().unwrap()
    } else {
        spine_dir
    };
    let algo = match config.repository.hash_algorithm.as_str() {
        "sha256" => HashAlgorithm::Sha256,
        _ => HashAlgorithm::Blake3,
    };
    let store = CompositeObjectStore::new(effective_spine_dir, algo, 3)
        .context("Failed to open object store")?;
    Ok(store)
}

#[derive(Clone, Debug)]
pub struct RepoState {
    pub head_ref_name: Option<String>,
    pub head_commit_id: Option<ObjectId>,
}

pub fn resolve_head(spine_dir: &Path) -> Result<RepoState> {
    let head_path = spine_dir.join("HEAD");
    if !head_path.exists() {
        bail!("fatal: corrupt repository: missing HEAD");
    }

    let head_content = fs::read_to_string(&head_path)
        .context("Failed to read HEAD")?
        .trim()
        .to_string();

    if let Some(target_ref) = head_content.strip_prefix("ref: ") {
        let branch_name = target_ref
            .strip_prefix("refs/heads/")
            .unwrap_or(target_ref)
            .to_string();
        let ref_path = spine_dir.join(target_ref);
        let ref_path = if ref_path.exists() {
            ref_path
        } else {
            // Also check primary spine dir if this is a linked worktree
            let prim_ref = spine_dir
                .parent()
                .and_then(|p| p.parent())
                .map(|p| p.join(target_ref));
            prim_ref.unwrap_or(ref_path)
        };

        if ref_path.exists() {
            let ref_content = fs::read_to_string(&ref_path)
                .context("Failed to read branch reference")?
                .trim()
                .to_string();
            let oid = ObjectId::from_hex(&ref_content).context("Invalid hash in ref file")?;
            Ok(RepoState {
                head_ref_name: Some(branch_name),
                head_commit_id: Some(oid),
            })
        } else {
            Ok(RepoState {
                head_ref_name: Some(branch_name),
                head_commit_id: None,
            })
        }
    } else {
        let oid = ObjectId::from_hex(&head_content).context("Invalid hash in detached HEAD")?;
        Ok(RepoState {
            head_ref_name: None,
            head_commit_id: Some(oid),
        })
    }
}

pub fn peel_to_commit(store: &CompositeObjectStore, mut oid: ObjectId) -> Result<ObjectId> {
    loop {
        let (obj_type, payload) = store.read_object(&oid)?;
        match obj_type {
            ObjectType::Commit => return Ok(oid),
            ObjectType::Tag => {
                let tag = spine_format::deserialize_annotated_tag(&payload)?;
                oid = tag.target_object;
            }
            _ => return Ok(oid),
        }
    }
}

pub fn resolve_commit_ref_advanced(
    spine_dir: &Path,
    primary_spine_dir: &Path,
    store: &CompositeObjectStore,
    rev: &str,
) -> Result<ObjectId> {
    let rev = rev.trim();
    if rev.is_empty() {
        bail!("fatal: empty revision");
    }

    // Handle HEAD~N, HEAD^, <rev>~N, <rev>^N
    if let Some(idx) = rev.find('~') {
        let base_rev = &rev[..idx];
        let count_str = &rev[idx + 1..];
        let count: usize = if count_str.is_empty() {
            1
        } else {
            count_str.parse().context("Invalid ~ count in revision")?
        };
        let mut current_oid =
            resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, base_rev)?;
        for _ in 0..count {
            let commit = get_commit(store, &current_oid)?;
            let parent = commit.parents.first().ok_or_else(|| {
                anyhow::anyhow!("Revision '{}' has no parent (reached root commit)", rev)
            })?;
            current_oid = *parent;
        }
        return Ok(current_oid);
    }

    if let Some(idx) = rev.find('^') {
        let base_rev = &rev[..idx];
        let parent_idx_str = &rev[idx + 1..];
        let parent_idx: usize = if parent_idx_str.is_empty() {
            1
        } else {
            parent_idx_str.parse().context("Invalid ^ parent index")?
        };
        if parent_idx == 0 {
            return resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, base_rev);
        }
        let base_oid = resolve_commit_ref_advanced(spine_dir, primary_spine_dir, store, base_rev)?;
        let commit = get_commit(store, &base_oid)?;
        let parent = commit.parents.get(parent_idx - 1).ok_or_else(|| {
            anyhow::anyhow!("Revision '{}' does not have parent #{}", rev, parent_idx)
        })?;
        return Ok(*parent);
    }

    if rev == "HEAD" {
        let state = resolve_head(spine_dir)?;
        return state
            .head_commit_id
            .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"));
    }

    // Check branch
    for dir in &[spine_dir, primary_spine_dir] {
        let branch_ref = dir.join("refs").join("heads").join(rev);
        if branch_ref.exists() {
            let content = fs::read_to_string(branch_ref)?.trim().to_string();
            let oid = ObjectId::from_hex(&content).context("Invalid branch ref")?;
            return peel_to_commit(store, oid);
        }
    }

    // Check tag
    for dir in &[spine_dir, primary_spine_dir] {
        let tag_ref = dir.join("refs").join("tags").join(rev);
        if tag_ref.exists() {
            let content = fs::read_to_string(tag_ref)?.trim().to_string();
            let oid = ObjectId::from_hex(&content).context("Invalid tag ref")?;
            return peel_to_commit(store, oid);
        }
    }

    // Check remote tracking ref (e.g. origin/main)
    for dir in &[spine_dir, primary_spine_dir] {
        let remote_ref = dir.join("refs").join("remotes").join(rev);
        if remote_ref.exists() {
            let content = fs::read_to_string(remote_ref)?.trim().to_string();
            let oid = ObjectId::from_hex(&content).context("Invalid remote tracking ref")?;
            return peel_to_commit(store, oid);
        }
    }

    // Direct 64-char hash
    if let Ok(oid) = ObjectId::from_hex(rev) {
        if store.has_object(&oid) {
            return peel_to_commit(store, oid);
        }
    }

    // Unique prefix matching (at least 4 chars)
    if rev.len() >= 4 && rev.chars().all(|c| c.is_ascii_hexdigit()) {
        let prefix = rev.to_ascii_lowercase();
        let loose = store.loose_store().list_objects()?;
        let mut matches = Vec::new();
        for oid in loose {
            if oid.to_hex().starts_with(&prefix) {
                matches.push(oid);
            }
        }
        if matches.len() == 1 {
            return peel_to_commit(store, matches[0]);
        } else if matches.len() > 1 {
            bail!(
                "fatal: ambiguous revision prefix '{}' ({} matches)",
                rev,
                matches.len()
            );
        }
    }

    bail!("fatal: Cannot resolve revision '{}'", rev)
}

pub fn resolve_commit_ref(
    spine_dir: &Path,
    store: &CompositeObjectStore,
    rev: &str,
) -> Result<ObjectId> {
    resolve_commit_ref_advanced(spine_dir, spine_dir, store, rev)
}

pub fn update_ref(spine_dir: &Path, target_ref: &str, new_oid: &ObjectId) -> Result<()> {
    let effective_spine_dir = if spine_dir
        .parent()
        .and_then(|p| p.file_name())
        .map_or(false, |n| n == "worktrees")
    {
        spine_dir.parent().unwrap().parent().unwrap()
    } else {
        spine_dir
    };

    let ref_path = effective_spine_dir.join(target_ref);
    let parent = ref_path.parent().unwrap();
    fs::create_dir_all(parent)?;

    let tmp_path = effective_spine_dir
        .join("tmp")
        .join(format!("ref-{}", Uuid::new_v4()));
    fs::create_dir_all(tmp_path.parent().unwrap())?;

    {
        let mut f = File::create(&tmp_path)?;
        writeln!(f, "{}", new_oid.to_hex())?;
        f.sync_all()?;
    }

    if let Err(e) = fs::rename(&tmp_path, &ref_path) {
        let _ = fs::remove_file(&ref_path);
        fs::rename(&tmp_path, &ref_path)
            .with_context(|| format!("Failed to update ref '{}': {}", target_ref, e))?;
    }

    Ok(())
}

pub fn capture_snapshot(
    spine_dir: &Path,
    state: &RepoState,
    store: &CompositeObjectStore,
) -> Result<RepoStateSnapshot> {
    let index_path = spine_dir.join("index");
    let index_checksum = if index_path.exists() {
        if let Ok(index) = Index::load_from_file(&index_path) {
            format!("entries:{}", index.len())
        } else {
            "none".to_string()
        }
    } else {
        "none".to_string()
    };

    let tree_id = match state.head_commit_id {
        Some(ref cid) => get_commit(store, cid).ok().map(|c| c.root_tree),
        None => None,
    };

    let head_ref = match &state.head_ref_name {
        Some(b) => format!("refs/heads/{}", b),
        None => format!(
            "detached:{}",
            state.head_commit_id.map(|o| o.to_hex()).unwrap_or_default()
        ),
    };

    Ok(RepoStateSnapshot {
        head_ref,
        branch_name: state.head_ref_name.clone(),
        branch_tip: state.head_commit_id,
        index_checksum,
        tree_id,
        worktree_id: None,
    })
}

fn restore_snapshot(
    worktree_root: &Path,
    spine_dir: &Path,
    store: &CompositeObjectStore,
    snapshot: &RepoStateSnapshot,
) -> Result<()> {
    // 1. Determine current tree BEFORE changing refs
    let current_state = resolve_head(spine_dir)?;
    let cur_tree = current_state
        .head_commit_id
        .and_then(|cid| get_commit(store, &cid).ok().map(|c| c.root_tree));

    let index_path = spine_dir.join("index");
    let mut index = Index::load_from_file(&index_path).unwrap_or_default();

    // 2. Restore tree if available
    if let Some(ref target_tree) = snapshot.tree_id {
        let _ = execute_switch(
            worktree_root,
            &mut index,
            store,
            cur_tree.as_ref(),
            target_tree,
        );
    } else if let Some(ref cur_t) = cur_tree {
        let mut cur_files = BTreeMap::new();
        let _ = flatten_tree(store, cur_t, "", &mut cur_files);
        for (path, _) in cur_files {
            let _ = fs::remove_file(worktree_root.join(&path));
            index.remove(&path);
        }
    }
    let _ = index.save_to_file(&index_path);

    // 3. Restore HEAD & branch ref
    if let Some(ref branch) = snapshot.branch_name {
        fs::write(
            spine_dir.join("HEAD"),
            format!("ref: refs/heads/{}\n", branch),
        )?;
        if let Some(tip) = snapshot.branch_tip {
            let ref_path = spine_dir.join("refs").join("heads").join(branch);
            if let Some(p) = ref_path.parent() {
                fs::create_dir_all(p)?;
            }
            fs::write(ref_path, format!("{}\n", tip.to_hex()))?;
        } else {
            let ref_path = spine_dir.join("refs").join("heads").join(branch);
            let _ = fs::remove_file(ref_path);
        }
    } else if let Some(tip) = snapshot.branch_tip {
        fs::write(spine_dir.join("HEAD"), format!("{}\n", tip.to_hex()))?;
    }

    Ok(())
}

fn list_branches(spine_dir: &Path) -> Result<Vec<(String, Option<ObjectId>)>> {
    let heads_dir = spine_dir.join("refs").join("heads");
    if !heads_dir.exists() {
        return Ok(Vec::new());
    }

    let mut branches = Vec::new();
    for entry in fs::read_dir(heads_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() {
            let name = path.file_name().unwrap().to_string_lossy().to_string();
            let content = fs::read_to_string(&path)?.trim().to_string();
            let oid = ObjectId::from_hex(&content).ok();
            branches.push((name, oid));
        }
    }

    branches.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(branches)
}

fn validate_branch_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("Branch name cannot be empty");
    }
    if name.contains(' ')
        || name.contains("..")
        || name.contains('~')
        || name.contains('^')
        || name.contains(':')
        || name.contains('\\')
    {
        bail!(
            "Invalid branch name '{}': contains forbidden characters",
            name
        );
    }
    if name.starts_with('-')
        || name.starts_with('/')
        || name.ends_with('/')
        || name.ends_with(".lock")
    {
        bail!("Invalid branch name '{}'", name);
    }
    Ok(())
}

pub fn summarize_diff<S: ObjectStore>(
    changes: &[spine_diff::TreeChange],
    store: &S,
) -> spine_diff::DiffSummary {
    let mut summary = spine_diff::DiffSummary {
        files_changed: changes.len(),
        ..Default::default()
    };

    for ch in changes {
        let (path, (old_data, new_data)) = match ch {
            spine_diff::TreeChange::Added { path, object_id } => {
                let data = store
                    .read_object(object_id)
                    .map(|(_, d)| d)
                    .unwrap_or_default();
                (path.clone(), (Vec::new(), data))
            }
            spine_diff::TreeChange::Deleted { path, object_id } => {
                let data = store
                    .read_object(object_id)
                    .map(|(_, d)| d)
                    .unwrap_or_default();
                (path.clone(), (data, Vec::new()))
            }
            spine_diff::TreeChange::Modified {
                path,
                old_id,
                new_id,
                ..
            } => {
                let d1 = store
                    .read_object(old_id)
                    .map(|(_, d)| d)
                    .unwrap_or_default();
                let d2 = store
                    .read_object(new_id)
                    .map(|(_, d)| d)
                    .unwrap_or_default();
                (path.clone(), (d1, d2))
            }
        };

        if spine_diff::is_binary(&old_data) || spine_diff::is_binary(&new_data) {
            summary.file_stats.push((path, 0, 0, true));
        } else {
            let s1 = String::from_utf8_lossy(&old_data);
            let s2 = String::from_utf8_lossy(&new_data);
            let ops = compute_line_diff(&s1, &s2);
            let mut adds = 0;
            let mut dels = 0;
            for op in ops {
                match op {
                    spine_diff::DiffOp::Addition(_) => adds += 1,
                    spine_diff::DiffOp::Deletion(_) => dels += 1,
                    spine_diff::DiffOp::Context(_) => {}
                }
            }
            summary.insertions += adds;
            summary.deletions += dels;
            summary.file_stats.push((path, adds, dels, false));
        }
    }

    summary
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Init { path } => {
            let target_dir = match path {
                Some(p) => {
                    if p.is_absolute() {
                        p
                    } else {
                        env::current_dir()?.join(p)
                    }
                }
                None => env::current_dir()?,
            };

            let spine_dir = target_dir.join(".spine");
            if spine_dir.exists() {
                bail!(
                    "fatal: destination path '{}' already contains a .spine repository",
                    target_dir.display()
                );
            }

            fs::create_dir_all(&spine_dir)?;
            fs::create_dir_all(spine_dir.join("objects"))?;
            fs::create_dir_all(spine_dir.join("refs").join("heads"))?;
            fs::create_dir_all(spine_dir.join("journal").join("records"))?;
            fs::create_dir_all(spine_dir.join("tmp"))?;

            let config = SpineConfig::default();
            config.save_to_file(spine_dir.join("spine.toml"))?;

            fs::write(spine_dir.join("HEAD"), "ref: refs/heads/main\n")?;

            println!(
                "Initialized empty Spine repository in {}",
                spine_dir.display()
            );
        }

        Commands::Config { action } => {
            let (_, spine_dir) = find_repo_root()?;
            let config_path = spine_dir.join("spine.toml");
            let mut config = SpineConfig::load_from_file(&config_path)?;

            match action {
                ConfigAction::Get { key } => match config.get_value(&key) {
                    Some(val) => println!("{}", val),
                    None => bail!("Configuration key '{}' not found", key),
                },
                ConfigAction::Set { key, value } => {
                    let has_objects = spine_dir.join("objects").exists()
                        && fs::read_dir(spine_dir.join("objects"))?.any(|e| {
                            e.map(|entry| entry.file_name().to_string_lossy().len() == 2)
                                .unwrap_or(false)
                        });

                    config.set_value_with_repo_check(&key, &value, has_objects)?;
                    config.save_to_file(&config_path)?;
                }
            }
        }

        Commands::Add { paths } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let mut index = Index::load_from_file(&index_path)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);

            let mut total_staged = 0;
            for p in &paths {
                total_staged += stage_path(&worktree_root, p, &mut index, &store, &ignore)?;
            }

            // If a merge is in progress, mark resolved conflicts
            if let Ok(Some(mut ms)) = MergeState::load_from_dir(&spine_dir) {
                let mut to_resolve = Vec::new();
                for p in &paths {
                    let p_str = p.to_string_lossy();
                    let p_norm = p_str.replace('\\', "/");
                    for c in &ms.conflicts {
                        if p_norm == "."
                            || p_norm == c.path
                            || c.path
                                .starts_with(&format!("{}/", p_norm.trim_end_matches('/')))
                        {
                            to_resolve.push(c.path.clone());
                        }
                    }
                }
                let mut updated = false;
                for path in to_resolve {
                    if ms.mark_resolved(&path) {
                        updated = true;
                    }
                }
                if updated {
                    ms.save_to_dir(&spine_dir)?;
                }
            }

            index.save_to_file(&index_path)?;
            if total_staged == 0 {
                println!("No changes staged.");
            }
        }

        Commands::Status => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let index = Index::load_from_file(&index_path)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);
            let state = resolve_head(&spine_dir)?;

            if let Some(ref branch) = state.head_ref_name {
                println!("On branch {}", branch);
            } else {
                println!("HEAD detached at {}", state.head_commit_id.unwrap());
            }

            // Check merge state
            if let Ok(Some(ms)) = MergeState::load_from_dir(&spine_dir) {
                println!("*** You have unmerged paths. ***");
                println!(
                    "  (fix conflicts and run \"spine commit\" or \"spine merge --continue\")"
                );
                println!("  (use \"spine merge --abort\" to abort the merge)\n");
                let unresolved = ms.unresolved_conflicts();
                if !unresolved.is_empty() {
                    println!("Unmerged paths:");
                    for c in unresolved {
                        println!("\tboth modified:   {}", c.path);
                    }
                    println!();
                }
                if !ms.resolved_paths.is_empty() {
                    println!("Resolved conflicts staged for commit:");
                    for path in &ms.resolved_paths {
                        println!("\tresolved:        {}", path);
                    }
                    println!();
                }
            }

            let mut head_tree_id = None;
            if let Some(cid) = state.head_commit_id {
                let commit = get_commit(&store, &cid)?;
                head_tree_id = Some(commit.root_tree);
            } else {
                println!("\nNo commits yet\n");
            }

            let report = compute_status(
                &worktree_root,
                &index,
                &store,
                head_tree_id.as_ref(),
                &ignore,
            )?;

            let has_staged = !report.staged_new.is_empty()
                || !report.staged_modified.is_empty()
                || !report.staged_deleted.is_empty();

            if has_staged {
                println!("Changes to be committed:");
                println!("  (use \"spine commit\" to record changes)\n");
                for f in &report.staged_new {
                    println!("\tnew file:   {}", f);
                }
                for f in &report.staged_modified {
                    println!("\tmodified:   {}", f);
                }
                for f in &report.staged_deleted {
                    println!("\tdeleted:    {}", f);
                }
                println!();
            }

            let has_unstaged =
                !report.unstaged_modified.is_empty() || !report.unstaged_deleted.is_empty();
            if has_unstaged {
                println!("Changes not staged for commit:");
                println!("  (use \"spine add <file>...\" to update what will be committed)\n");
                for f in &report.unstaged_modified {
                    println!("\tmodified:   {}", f);
                }
                for f in &report.unstaged_deleted {
                    println!("\tdeleted:    {}", f);
                }
                println!();
            }

            if !report.untracked.is_empty() {
                println!("Untracked files:");
                println!("  (use \"spine add <file>...\" to include in what will be committed)\n");
                for f in &report.untracked {
                    println!("\t{}", f);
                }
                println!();
            }

            if report.is_clean() && !spine_dir.join("MERGE_STATE").exists() {
                println!("nothing to commit, working tree clean");
            }
        }

        Commands::Commit { message } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let index = Index::load_from_file(&index_path)?;
            let state = resolve_head(&spine_dir)?;

            if index.is_empty() {
                bail!("nothing to commit (create/copy files and use \"spine add\" to track)");
            }

            let mut journal = Journal::new(spine_dir.join("journal"))?;
            let _ = journal.check_and_recover()?;
            let before_snapshot = capture_snapshot(&spine_dir, &state, &store)?;

            let author_name = config
                .user
                .name
                .or_else(|| env::var("SPINE_AUTHOR_NAME").ok())
                .unwrap_or_else(|| "Anonymous Developer".to_string());

            let author_email = config
                .user
                .email
                .or_else(|| env::var("SPINE_AUTHOR_EMAIL").ok())
                .unwrap_or_else(|| "anonymous@spine.local".to_string());

            let committer_name = author_name.clone();
            let committer_email = author_email.clone();

            let root_tree_id = write_index_to_tree(&index, &store)?;

            let mut parents = Vec::new();
            if let Some(parent_cid) = state.head_commit_id {
                let parent_commit = get_commit(&store, &parent_cid)?;
                // Only skip if not completing a merge
                let merge_state_opt = MergeState::load_from_dir(&spine_dir)?;
                if merge_state_opt.is_none() && parent_commit.root_tree == root_tree_id {
                    println!("nothing to commit, working tree clean");
                    return Ok(());
                }
                parents.push(parent_cid);
            }

            // Check if completing a merge
            let pending_merge = MergeState::load_from_dir(&spine_dir)?;
            if let Some(ref ms) = pending_merge {
                if !ms.is_fully_resolved() {
                    let unresolved = ms.unresolved_conflicts();
                    let first = unresolved
                        .first()
                        .map(|c| c.path.as_str())
                        .unwrap_or("unknown");
                    bail!(
                        "fatal: Exiting because of unresolved conflict in '{}'. Stage resolved files with 'spine add <file>' before committing.",
                        first
                    );
                }
                // Ensure second parent is added
                if parents.len() == 1 && parents[0] == ms.ours_commit {
                    parents.push(ms.theirs_commit);
                }
            }

            let tx = journal.prepare_transaction(
                OperationKind::Commit,
                format!("Commit: {}", message.trim()),
                before_snapshot,
                None,
            )?;

            let now = Utc::now();
            let timestamp_nanos = now.timestamp_nanos_opt().unwrap_or(0);

            let commit = Commit {
                format_version: 2,
                root_tree: root_tree_id,
                parents,
                author_name,
                author_email,
                committer_name,
                committer_email,
                timestamp_nanos,
                message: message.trim().to_string(),
            };

            let commit_bytes = serialize_commit(&commit);
            let commit_id = store.write_object(ObjectType::Commit, &commit_bytes)?;

            // Update branch reference or detached HEAD
            if let Some(ref branch) = state.head_ref_name {
                let ref_path_rel = format!("refs/heads/{}", branch);
                update_ref(&spine_dir, &ref_path_rel, &commit_id)?;
                let short_id = &commit_id.to_hex()[..7];
                let desc = if commit.is_merge() {
                    "(merge-commit) "
                } else if commit.is_root() {
                    "(root-commit) "
                } else {
                    ""
                };
                println!("[{} {}{}] {}", branch, desc, short_id, commit.message);
            } else {
                fs::write(spine_dir.join("HEAD"), format!("{}\n", commit_id.to_hex()))?;
                let short_id = &commit_id.to_hex()[..7];
                println!("[detached HEAD {}] {}", short_id, commit.message);
            }

            // Clear merge state if it was a merge
            if pending_merge.is_some() {
                MergeState::clear(&spine_dir)?;
            }

            let new_state = resolve_head(&spine_dir)?;
            let after_snapshot = capture_snapshot(&spine_dir, &new_state, &store)?;
            journal.commit_transaction(&tx.id, after_snapshot)?;
        }

        Commands::Log { limit } => {
            let (_, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let state = resolve_head(&spine_dir)?;

            let head_cid = match state.head_commit_id {
                Some(cid) => cid,
                None => {
                    println!(
                        "No commits yet on branch {}",
                        state.head_ref_name.unwrap_or_else(|| "HEAD".into())
                    );
                    return Ok(());
                }
            };

            let records = walk_history(&store, &head_cid, limit)?;
            for (idx, record) in records.iter().enumerate() {
                let ref_indicator = if idx == 0 {
                    if let Some(ref b) = state.head_ref_name {
                        format!(" (HEAD -> {})", b)
                    } else {
                        " (HEAD)".to_string()
                    }
                } else {
                    String::new()
                };

                println!("commit {}{}", record.id, ref_indicator);
                if record.commit.is_merge() {
                    let parents_str: Vec<String> = record
                        .commit
                        .parents
                        .iter()
                        .map(|p| p.to_hex()[..7].to_string())
                        .collect();
                    println!("Merge: {}", parents_str.join(" "));
                }
                println!(
                    "Author: {} <{}>",
                    record.commit.author_name, record.commit.author_email
                );
                let dt = Utc.timestamp_nanos(record.commit.timestamp_nanos);
                println!("Date:   {}", dt.to_rfc2822());
                println!();
                for line in record.commit.message.lines() {
                    println!("    {}", line);
                }
                println!();
            }
        }

        Commands::Show { object } => {
            let (_, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;

            let target_cid = resolve_commit_ref(&spine_dir, &store, &object)?;
            let commit = get_commit(&store, &target_cid)?;
            println!("commit {}", target_cid);
            if commit.is_merge() {
                let p_str: Vec<String> = commit
                    .parents
                    .iter()
                    .map(|p| p.to_hex()[..7].to_string())
                    .collect();
                println!("Merge: {}", p_str.join(" "));
            }
            println!("Author: {} <{}>", commit.author_name, commit.author_email);
            let dt = Utc.timestamp_nanos(commit.timestamp_nanos);
            println!("Date:   {}", dt.to_rfc2822());
            println!("Tree:   {}", commit.root_tree);
            println!();
            for line in commit.message.lines() {
                println!("    {}", line);
            }
            println!();

            let mut tree_files = BTreeMap::new();
            flatten_tree(&store, &commit.root_tree, "", &mut tree_files)?;
            println!("Tree contents ({} files):", tree_files.len());
            for (path, (mode, oid)) in tree_files {
                println!(
                    "  {:06o} blob {}  {}",
                    mode.as_u32(),
                    &oid.to_hex()[..8],
                    path
                );
            }
        }

        Commands::Branch { action } => {
            let (_, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let state = resolve_head(&spine_dir)?;
            let mut journal = Journal::new(spine_dir.join("journal"))?;

            let action = action.unwrap_or(BranchAction::List);

            match action {
                BranchAction::List => {
                    let branches = list_branches(&spine_dir)?;
                    let current_branch = state.head_ref_name.as_deref().unwrap_or("");
                    for (b_name, _oid) in branches {
                        if b_name == current_branch {
                            println!("* {}", b_name);
                        } else {
                            println!("  {}", b_name);
                        }
                    }
                }
                BranchAction::Create { name, from } => {
                    validate_branch_name(&name)?;
                    let branch_ref_path = spine_dir.join("refs").join("heads").join(&name);
                    if branch_ref_path.exists() {
                        bail!("fatal: A branch named '{}' already exists.", name);
                    }

                    let target_cid = match from {
                        Some(ref rev) => resolve_commit_ref(&spine_dir, &store, rev)?,
                        None => state.head_commit_id.ok_or_else(|| {
                            anyhow::anyhow!("fatal: Not a valid object name: 'HEAD'.")
                        })?,
                    };

                    let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    let tx = journal.prepare_transaction(
                        OperationKind::BranchCreate,
                        format!("Create branch {}", name),
                        before_snap,
                        None,
                    )?;

                    update_ref(&spine_dir, &format!("refs/heads/{}", name), &target_cid)?;

                    let after_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    journal.commit_transaction(&tx.id, after_snap)?;
                }
                BranchAction::Delete { name, force: _ } => {
                    if Some(&name) == state.head_ref_name.as_ref() {
                        bail!(
                            "fatal: Cannot delete the branch '{}' which you are currently on.",
                            name
                        );
                    }

                    let branch_ref_path = spine_dir.join("refs").join("heads").join(&name);
                    if !branch_ref_path.exists() {
                        bail!("error: branch '{}' not found.", name);
                    }

                    let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    let tx = journal.prepare_transaction(
                        OperationKind::BranchDelete,
                        format!("Delete branch {}", name),
                        before_snap,
                        None,
                    )?;

                    fs::remove_file(&branch_ref_path)?;

                    let after_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    journal.commit_transaction(&tx.id, after_snap)?;
                    println!("Deleted branch {}.", name);
                }
                BranchAction::Rename { old_name, new_name } => {
                    validate_branch_name(&new_name)?;
                    let old_ref_path = spine_dir.join("refs").join("heads").join(&old_name);
                    if !old_ref_path.exists() {
                        bail!("error: ref 'refs/heads/{}' not found.", old_name);
                    }
                    let new_ref_path = spine_dir.join("refs").join("heads").join(&new_name);
                    if new_ref_path.exists() {
                        bail!("fatal: A branch named '{}' already exists.", new_name);
                    }

                    let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    let tx = journal.prepare_transaction(
                        OperationKind::BranchRename,
                        format!("Rename branch {} -> {}", old_name, new_name),
                        before_snap,
                        None,
                    )?;

                    fs::rename(&old_ref_path, &new_ref_path)?;

                    if Some(&old_name) == state.head_ref_name.as_ref() {
                        fs::write(
                            spine_dir.join("HEAD"),
                            format!("ref: refs/heads/{}\n", new_name),
                        )?;
                    }

                    let new_state = resolve_head(&spine_dir)?;
                    let after_snap = capture_snapshot(&spine_dir, &new_state, &store)?;
                    journal.commit_transaction(&tx.id, after_snap)?;
                }
            }
        }

        Commands::Switch { branch, create } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let mut index = Index::load_from_file(&index_path)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);
            let state = resolve_head(&spine_dir)?;
            let mut journal = Journal::new(spine_dir.join("journal"))?;

            let mut created_new = false;
            let target_ref_path = spine_dir.join("refs").join("heads").join(&branch);

            if create {
                validate_branch_name(&branch)?;
                if target_ref_path.exists() {
                    bail!("fatal: A branch named '{}' already exists.", branch);
                }
                let head_cid = state.head_commit_id.ok_or_else(|| {
                    anyhow::anyhow!("fatal: Cannot create branch '{}' from unborn HEAD", branch)
                })?;
                update_ref(&spine_dir, &format!("refs/heads/{}", branch), &head_cid)?;
                created_new = true;
            } else if !target_ref_path.exists() {
                bail!("fatal: invalid reference: {}", branch);
            }

            if Some(&branch) == state.head_ref_name.as_ref() {
                println!("Already on '{}'", branch);
                return Ok(());
            }

            let target_cid_str = fs::read_to_string(&target_ref_path)?.trim().to_string();
            let target_cid = ObjectId::from_hex(&target_cid_str)?;
            let target_commit = get_commit(&store, &target_cid)?;

            let cur_tree_id = match state.head_commit_id {
                Some(cid) => Some(get_commit(&store, &cid)?.root_tree),
                None => None,
            };

            // Preflight safety check
            check_switch_safety(
                &worktree_root,
                &index,
                &store,
                cur_tree_id.as_ref(),
                &target_commit.root_tree,
                &ignore,
            )?;

            let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
            let tx = journal.prepare_transaction(
                OperationKind::Switch,
                format!("Switch to branch {}", branch),
                before_snap,
                None,
            )?;

            // Safe working tree switch
            execute_switch(
                &worktree_root,
                &mut index,
                &store,
                cur_tree_id.as_ref(),
                &target_commit.root_tree,
            )?;
            index.save_to_file(&index_path)?;

            // Update HEAD
            fs::write(
                spine_dir.join("HEAD"),
                format!("ref: refs/heads/{}\n", branch),
            )?;

            let new_state = resolve_head(&spine_dir)?;
            let after_snap = capture_snapshot(&spine_dir, &new_state, &store)?;
            journal.commit_transaction(&tx.id, after_snap)?;

            if created_new {
                println!("Switched to a new branch '{}'", branch);
            } else {
                println!("Switched to branch '{}'", branch);
            }
        }

        Commands::Diff {
            staged,
            stat,
            rev_a,
            rev_b,
        } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let index = Index::load_from_file(&index_path)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);
            let state = resolve_head(&spine_dir)?;

            if let (Some(a), Some(b)) = (rev_a, rev_b) {
                // Two revisions comparison
                let cid_a = resolve_commit_ref(&spine_dir, &store, &a)?;
                let cid_b = resolve_commit_ref(&spine_dir, &store, &b)?;
                let c_a = get_commit(&store, &cid_a)?;
                let c_b = get_commit(&store, &cid_b)?;

                let mut files_a = BTreeMap::new();
                flatten_tree(&store, &c_a.root_tree, "", &mut files_a)?;
                let mut files_b = BTreeMap::new();
                flatten_tree(&store, &c_b.root_tree, "", &mut files_b)?;

                let changes = compare_trees(&files_a, &files_b);
                if stat {
                    let s = summarize_diff(&changes, &store);
                    print!("{}", format_diff_stat(&s));
                } else {
                    for ch in &changes {
                        let path = ch.path();
                        let (old_content, new_content) = match ch {
                            spine_diff::TreeChange::Added { object_id, .. } => {
                                let (_, b) = store.read_object(object_id)?;
                                (String::new(), String::from_utf8_lossy(&b).to_string())
                            }
                            spine_diff::TreeChange::Deleted { object_id, .. } => {
                                let (_, b) = store.read_object(object_id)?;
                                (String::from_utf8_lossy(&b).to_string(), String::new())
                            }
                            spine_diff::TreeChange::Modified { old_id, new_id, .. } => {
                                let (_, b1) = store.read_object(old_id)?;
                                let (_, b2) = store.read_object(new_id)?;
                                (
                                    String::from_utf8_lossy(&b1).to_string(),
                                    String::from_utf8_lossy(&b2).to_string(),
                                )
                            }
                        };
                        print!("{}", format_unified_diff(path, &old_content, &new_content));
                    }
                }
            } else if staged {
                // Staged vs HEAD commit
                let head_tree = state
                    .head_commit_id
                    .map(|cid| get_commit(&store, &cid).map(|c| c.root_tree))
                    .transpose()?;
                let mut head_files = BTreeMap::new();
                if let Some(ref t) = head_tree {
                    flatten_tree(&store, t, "", &mut head_files)?;
                }

                let mut staged_files = BTreeMap::new();
                for entry in index.iter() {
                    staged_files.insert(entry.path.clone(), (entry.mode, entry.object_id));
                }

                let changes = compare_trees(&head_files, &staged_files);

                if stat {
                    let s = summarize_diff(&changes, &store);
                    print!("{}", format_diff_stat(&s));
                } else {
                    for ch in &changes {
                        let path = ch.path();
                        let (old_c, new_c) = match ch {
                            spine_diff::TreeChange::Added { object_id, .. } => {
                                let (_, b) = store.read_object(object_id)?;
                                (String::new(), String::from_utf8_lossy(&b).to_string())
                            }
                            spine_diff::TreeChange::Deleted { object_id, .. } => {
                                let (_, b) = store.read_object(object_id)?;
                                (String::from_utf8_lossy(&b).to_string(), String::new())
                            }
                            spine_diff::TreeChange::Modified { old_id, new_id, .. } => {
                                let (_, b1) = store.read_object(old_id)?;
                                let (_, b2) = store.read_object(new_id)?;
                                (
                                    String::from_utf8_lossy(&b1).to_string(),
                                    String::from_utf8_lossy(&b2).to_string(),
                                )
                            }
                        };
                        print!("{}", format_unified_diff(path, &old_c, &new_c));
                    }
                }
            } else {
                // Unstaged changes (Disk vs Index)
                let rep = compute_status(&worktree_root, &index, &store, None, &ignore)?;

                for path in &rep.unstaged_modified {
                    let disk_path = worktree_root.join(path);
                    let disk_content = fs::read_to_string(&disk_path).unwrap_or_default();
                    let idx_entry = index.get(path).unwrap();
                    let (_, blob_content) = store.read_object(&idx_entry.object_id)?;
                    let idx_str = String::from_utf8_lossy(&blob_content);

                    print!("{}", format_unified_diff(path, &idx_str, &disk_content));
                }
            }
        }

        Commands::Merge {
            branch,
            abort,
            continue_merge,
            no_ff,
            message,
        } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let mut index = Index::load_from_file(&index_path)?;
            let state = resolve_head(&spine_dir)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);
            let mut journal = Journal::new(spine_dir.join("journal"))?;

            if abort {
                let ms = MergeState::load_from_dir(&spine_dir)?
                    .ok_or_else(|| anyhow::anyhow!("fatal: There is no merge to abort"))?;

                let ours_commit = get_commit(&store, &ms.ours_commit)?;

                let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                let tx = journal.prepare_transaction(
                    OperationKind::MergeAbort,
                    "Abort merge".to_string(),
                    before_snap,
                    None,
                )?;

                // Restore all files directly from ours_commit tree
                let mut ours_files = BTreeMap::new();
                flatten_tree(&store, &ours_commit.root_tree, "", &mut ours_files)?;

                for (path, (mode, oid)) in &ours_files {
                    let full = worktree_root.join(path);
                    if let Some(p) = full.parent() {
                        fs::create_dir_all(p)?;
                    }
                    let (_, content) = store.read_object(oid)?;
                    fs::write(&full, content)?;
                    let meta = fs::metadata(&full)?;
                    let (_mode, size, s, n) = metadata_to_stat(&meta);
                    index.add_or_update(IndexEntry {
                        path: path.clone(),
                        mode: *mode,
                        file_size: size,
                        mtime_secs: s,
                        mtime_nanos: n,
                        inode: 0,
                        object_id: *oid,
                        stage_status: StageStatus::Clean,
                    });
                }
                index.save_to_file(&index_path)?;
                MergeState::clear(&spine_dir)?;

                let after_snap = capture_snapshot(&spine_dir, &state, &store)?;
                journal.commit_transaction(&tx.id, after_snap)?;
                println!("Merge aborted: restored working tree to previous state.");
                return Ok(());
            }

            if continue_merge {
                let ms = MergeState::load_from_dir(&spine_dir)?.ok_or_else(|| {
                    anyhow::anyhow!("fatal: There is no merge in progress (MERGE_STATE missing).")
                })?;

                // Check for unresolved conflicts via explicit state tracking
                if !ms.is_fully_resolved() {
                    let unresolved = ms.unresolved_conflicts();
                    let first = unresolved
                        .first()
                        .map(|c| c.path.as_str())
                        .unwrap_or("unknown");
                    bail!(
                        "fatal: Exiting because of unresolved conflict in '{}'. Use 'spine add <file>' after resolving.",
                        first
                    );
                }

                let root_tree_id = write_index_to_tree(&index, &store)?;
                let author_name = config
                    .user
                    .name
                    .unwrap_or_else(|| "Anonymous Developer".to_string());
                let author_email = config
                    .user
                    .email
                    .unwrap_or_else(|| "anonymous@spine.local".to_string());
                let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);

                let default_msg = format!("Merge branch '{}'", ms.source_branch);
                let commit_msg = message.unwrap_or(default_msg);

                let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                let tx = journal.prepare_transaction(
                    OperationKind::Commit,
                    format!("Merge commit: {}", commit_msg),
                    before_snap,
                    None,
                )?;

                let commit = Commit {
                    format_version: 2,
                    root_tree: root_tree_id,
                    parents: vec![ms.ours_commit, ms.theirs_commit],
                    author_name: author_name.clone(),
                    author_email: author_email.clone(),
                    committer_name: author_name,
                    committer_email: author_email,
                    timestamp_nanos: now,
                    message: commit_msg.clone(),
                };

                let c_bytes = serialize_commit(&commit);
                let cid = store.write_object(ObjectType::Commit, &c_bytes)?;

                if let Some(ref b) = state.head_ref_name {
                    update_ref(&spine_dir, &format!("refs/heads/{}", b), &cid)?;
                    println!(
                        "[{} (merge-commit) {}] {}",
                        b,
                        &cid.to_hex()[..7],
                        commit_msg
                    );
                }

                MergeState::clear(&spine_dir)?;
                let after_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
                journal.commit_transaction(&tx.id, after_snap)?;
                return Ok(());
            }

            let branch_name =
                branch.ok_or_else(|| anyhow::anyhow!("fatal: branch name required for merge"))?;
            let theirs_cid = resolve_commit_ref(&spine_dir, &store, &branch_name)?;
            let ours_cid = state
                .head_commit_id
                .ok_or_else(|| anyhow::anyhow!("fatal: HEAD has no commits yet"))?;

            if ours_cid == theirs_cid {
                println!("Already up to date.");
                return Ok(());
            }

            let theirs_commit = get_commit(&store, &theirs_cid)?;
            let ours_commit = get_commit(&store, &ours_cid)?;

            // Fast-forward check
            if is_ancestor(&store, &ours_cid, &theirs_cid)? {
                if !no_ff {
                    // Preflight safety check
                    check_switch_safety(
                        &worktree_root,
                        &index,
                        &store,
                        Some(&ours_commit.root_tree),
                        &theirs_commit.root_tree,
                        &ignore,
                    )?;

                    let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
                    let tx = journal.prepare_transaction(
                        OperationKind::Merge,
                        format!("Fast-forward merge to {}", branch_name),
                        before_snap,
                        None,
                    )?;

                    execute_switch(
                        &worktree_root,
                        &mut index,
                        &store,
                        Some(&ours_commit.root_tree),
                        &theirs_commit.root_tree,
                    )?;
                    index.save_to_file(&index_path)?;

                    if let Some(ref b) = state.head_ref_name {
                        update_ref(&spine_dir, &format!("refs/heads/{}", b), &theirs_cid)?;
                    }

                    let after_snap =
                        capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
                    journal.commit_transaction(&tx.id, after_snap)?;

                    println!(
                        "Updating {}..{}",
                        &ours_cid.to_hex()[..7],
                        &theirs_cid.to_hex()[..7]
                    );
                    println!("Fast-forward");
                    return Ok(());
                }
            } else if is_ancestor(&store, &theirs_cid, &ours_cid)? {
                println!("Already up to date.");
                return Ok(());
            }

            // Three-way merge
            let base_cid =
                find_best_merge_base(&store, &ours_cid, &theirs_cid)?.ok_or_else(|| {
                    anyhow::anyhow!("fatal: No common ancestor found between branches")
                })?;
            let base_commit = get_commit(&store, &base_cid)?;

            let mut base_files = BTreeMap::new();
            flatten_tree(&store, &base_commit.root_tree, "", &mut base_files)?;
            let mut ours_files = BTreeMap::new();
            flatten_tree(&store, &ours_commit.root_tree, "", &mut ours_files)?;
            let mut theirs_files = BTreeMap::new();
            flatten_tree(&store, &theirs_commit.root_tree, "", &mut theirs_files)?;

            let resolutions = merge_3way_trees(&store, &base_files, &ours_files, &theirs_files)?;

            let mut conflicts = Vec::new();

            for (path, res) in &resolutions {
                match res {
                    ResolvedPath::Conflict(c) => conflicts.push(c.clone()),
                    ResolvedPath::MergedClean {
                        mode,
                        object_id,
                        content,
                    } => {
                        let full = worktree_root.join(path);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(&full, content)?;
                        let meta = fs::metadata(&full)?;
                        let (_fmode, size, s, n) = metadata_to_stat(&meta);
                        index.add_or_update(IndexEntry {
                            path: path.clone(),
                            mode: *mode,
                            file_size: size,
                            mtime_secs: s,
                            mtime_nanos: n,
                            inode: 0,
                            object_id: *object_id,
                            stage_status: StageStatus::Clean,
                        });
                    }
                    ResolvedPath::TakeTheirs {
                        mode,
                        object_id,
                        content,
                    } => {
                        let full = worktree_root.join(path);
                        if let Some(p) = full.parent() {
                            fs::create_dir_all(p)?;
                        }
                        fs::write(&full, content)?;
                        let meta = fs::metadata(&full)?;
                        let (_fmode, size, s, n) = metadata_to_stat(&meta);
                        index.add_or_update(IndexEntry {
                            path: path.clone(),
                            mode: *mode,
                            file_size: size,
                            mtime_secs: s,
                            mtime_nanos: n,
                            inode: 0,
                            object_id: *object_id,
                            stage_status: StageStatus::Clean,
                        });
                    }
                    ResolvedPath::Delete => {
                        let full = worktree_root.join(path);
                        if full.exists() {
                            let _ = fs::remove_file(&full);
                        }
                        index.remove(path);
                    }
                    ResolvedPath::KeepOurs => {}
                }
            }

            if !conflicts.is_empty() {
                // Write conflict markers to working tree for text conflicts
                for c in &conflicts {
                    let full = worktree_root.join(&c.path);
                    match c.kind {
                        spine_merge::ConflictKind::ModifyModifyText { base, ours, theirs } => {
                            let (_, b_data) = store.read_object(&base)?;
                            let (_, o_data) = store.read_object(&ours)?;
                            let (_, t_data) = store.read_object(&theirs)?;
                            let b_s = String::from_utf8_lossy(&b_data);
                            let o_s = String::from_utf8_lossy(&o_data);
                            let t_s = String::from_utf8_lossy(&t_data);

                            if let TextMergeResult::Conflict(marked) =
                                merge_3way_text(&b_s, &o_s, &t_s)
                            {
                                fs::write(&full, marked)?;
                            }
                        }
                        spine_merge::ConflictKind::AddAdd { ours, theirs } => {
                            let (_, o_data) = store.read_object(&ours)?;
                            let (_, t_data) = store.read_object(&theirs)?;
                            let o_s = String::from_utf8_lossy(&o_data);
                            let t_s = String::from_utf8_lossy(&t_data);
                            if let TextMergeResult::Conflict(marked) =
                                merge_3way_text("", &o_s, &t_s)
                            {
                                fs::write(&full, marked)?;
                            }
                        }
                        _ => {}
                    }
                    println!("CONFLICT (content): Merge conflict in {}", c.path);
                }

                let ms = MergeState {
                    source_branch: branch_name.clone(),
                    target_branch: state
                        .head_ref_name
                        .clone()
                        .unwrap_or_else(|| "HEAD".to_string()),
                    base_commit: base_cid,
                    ours_commit: ours_cid,
                    theirs_commit: theirs_cid,
                    conflicts,
                    resolved_paths: std::collections::BTreeSet::new(),
                    timestamp_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                };
                ms.save_to_dir(&spine_dir)?;
                index.save_to_file(&index_path)?;

                println!("Automatic merge failed; fix conflicts and then commit the result.");
                return Ok(());
            }

            // All clean! Complete merge commit automatically
            index.save_to_file(&index_path)?;
            let root_tree_id = write_index_to_tree(&index, &store)?;
            let author_name = config
                .user
                .name
                .unwrap_or_else(|| "Anonymous Developer".to_string());
            let author_email = config
                .user
                .email
                .unwrap_or_else(|| "anonymous@spine.local".to_string());
            let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let default_msg = format!("Merge branch '{}'", branch_name);
            let commit_msg = message.unwrap_or(default_msg);

            let before_snap = capture_snapshot(&spine_dir, &state, &store)?;
            let tx = journal.prepare_transaction(
                OperationKind::Merge,
                format!("Merge branch {}", branch_name),
                before_snap,
                None,
            )?;

            let commit = Commit {
                format_version: 2,
                root_tree: root_tree_id,
                parents: vec![ours_cid, theirs_cid],
                author_name: author_name.clone(),
                author_email: author_email.clone(),
                committer_name: author_name,
                committer_email: author_email,
                timestamp_nanos: now,
                message: commit_msg.clone(),
            };

            let c_bytes = serialize_commit(&commit);
            let cid = store.write_object(ObjectType::Commit, &c_bytes)?;

            if let Some(ref b) = state.head_ref_name {
                update_ref(&spine_dir, &format!("refs/heads/{}", b), &cid)?;
                println!("Merge made by the 'three-way' strategy.");
                println!(
                    "[{} (merge-commit) {}] {}",
                    b,
                    &cid.to_hex()[..7],
                    commit_msg
                );
            }

            let after_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
            journal.commit_transaction(&tx.id, after_snap)?;
        }

        Commands::Undo { list } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let mut journal = Journal::new(spine_dir.join("journal"))?;

            if list {
                let records = journal.list_records()?;
                println!("Transaction History ({} operations):", records.len());
                for r in records {
                    let dt = Utc.timestamp_nanos(r.timestamp_nanos);
                    println!(
                        "  [{:04}] {} | {} | {:?} | {:?} | {}",
                        r.sequence,
                        &r.id[..8],
                        dt.format("%Y-%m-%d %H:%M:%S"),
                        r.kind,
                        r.status,
                        r.description
                    );
                }
                return Ok(());
            }

            let to_undo = journal.plan_undo()?;
            let before_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;

            let tx = journal.prepare_transaction(
                OperationKind::Undo,
                format!("Undo operation {}", to_undo.id),
                before_snap,
                Some(to_undo.id.clone()),
            )?;

            restore_snapshot(&worktree_root, &spine_dir, &store, &to_undo.before_state)?;

            let after_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
            journal.commit_transaction(&tx.id, after_snap)?;

            println!(
                "Undid operation {}: {}",
                &to_undo.id[..8],
                to_undo.description
            );
        }

        Commands::Redo => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let mut journal = Journal::new(spine_dir.join("journal"))?;

            let to_redo = journal.plan_redo()?;
            let after_state = to_redo
                .after_state
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Operation to redo has no recorded after-state"))?;

            let before_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
            let tx = journal.prepare_transaction(
                OperationKind::Redo,
                format!("Redo operation {}", to_redo.id),
                before_snap,
                Some(to_redo.id.clone()),
            )?;

            restore_snapshot(&worktree_root, &spine_dir, &store, after_state)?;

            let after_snap = capture_snapshot(&spine_dir, &resolve_head(&spine_dir)?, &store)?;
            journal.commit_transaction(&tx.id, after_snap)?;

            println!(
                "Redid operation {}: {}",
                &to_redo.id[..8],
                to_redo.description
            );
        }

        Commands::Fsck => {
            let (_, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;

            println!(
                "Verifying repository integrity at {}...",
                spine_dir.display()
            );
            let report = run_fsck(&spine_dir, &store)?;

            println!("Checked {} objects", report.objects_checked);
            println!("Checked {} references", report.refs_checked);
            println!("Checked {} journal records", report.journal_records_checked);

            if !report.corrupt_objects.is_empty() {
                eprintln!(
                    "\nERROR: Found {} corrupt objects:",
                    report.corrupt_objects.len()
                );
                for (oid, err) in &report.corrupt_objects {
                    eprintln!("  corrupt object {}: {}", oid, err);
                }
            }

            if !report.missing_objects.is_empty() {
                eprintln!(
                    "\nERROR: Found {} missing referenced objects:",
                    report.missing_objects.len()
                );
                for oid in &report.missing_objects {
                    eprintln!("  missing object {}", oid);
                }
            }

            if !report.invalid_refs.is_empty() {
                eprintln!(
                    "\nERROR: Found {} invalid references:",
                    report.invalid_refs.len()
                );
                for (name, err) in &report.invalid_refs {
                    eprintln!("  invalid ref '{}': {}", name, err);
                }
            }

            if !report.journal_errors.is_empty() {
                eprintln!(
                    "\nERROR: Found {} journal integrity errors:",
                    report.journal_errors.len()
                );
                for (id, err) in &report.journal_errors {
                    eprintln!("  journal entry {}: {}", id, err);
                }
            }

            if !report.merge_state_errors.is_empty() {
                eprintln!(
                    "\nERROR: Found {} merge state errors:",
                    report.merge_state_errors.len()
                );
                for err in &report.merge_state_errors {
                    eprintln!("  merge state: {}", err);
                }
            }

            if !report.dangling_objects.is_empty() {
                println!(
                    "\nNotice: Found {} dangling unreferenced objects:",
                    report.dangling_objects.len()
                );
                for oid in &report.dangling_objects {
                    println!("  dangling {}", oid);
                }
            }

            if report.is_healthy() {
                println!("\nIntegrity check complete: Repository is HEALTHY with zero errors.");
            } else {
                bail!("Repository verification FAILED. Corruption or missing references detected.");
            }
        }

        Commands::CountObjects => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let summary = store.count_objects();

            println!("Storage Engine V2 Object Count Summary:");
            println!(
                "  Loose objects:        {:>8} ({})",
                summary.loose_count,
                format_bytes(summary.loose_size_bytes)
            );
            println!("  Packfiles:            {:>8}", summary.pack_count);
            println!(
                "  Packed objects:       {:>8} ({})",
                summary.packed_count,
                format_bytes(summary.packed_size_bytes)
            );
            println!(
                "  Total objects:        {:>8}",
                summary.loose_count + summary.packed_count
            );
            println!(
                "  Total disk footprint: {:>8}",
                format_bytes(summary.loose_size_bytes + summary.packed_size_bytes)
            );
        }

        Commands::Pack { action } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;

            match action {
                PackAction::List => {
                    let catalog = store.catalog();
                    if catalog.active_packs.is_empty() {
                        println!("No active packfiles found in catalog.");
                    } else {
                        println!(
                            "Catalog Generation: {}, Active Packs: {}",
                            catalog.generation,
                            catalog.active_packs.len()
                        );
                        println!(
                            "{:<42} {:>10} {:>12} {:>12}  CHECKSUM",
                            "PACK ID", "OBJECTS", "PACK SIZE", "INDEX SIZE"
                        );
                        for p in &catalog.active_packs {
                            println!(
                                "{:<42} {:>10} {:>12} {:>12}  {:.12}...",
                                p.id,
                                p.object_count,
                                format_bytes(p.pack_size_bytes),
                                format_bytes(p.index_size_bytes),
                                p.pack_checksum
                            );
                        }
                    }
                }
                PackAction::Inspect { id } => {
                    let pack_dir = spine_dir.join("objects").join("pack");
                    let spk_path = pack_dir.join(format!("{}.spk", id));
                    let spidx_path = pack_dir.join(format!("{}.spidx", id));
                    if !spk_path.exists() {
                        bail!("Packfile not found: {}", spk_path.display());
                    }
                    let mut reader = PackReader::open(&spk_path)?;
                    let idx_reader = PackIndexReader::open(&spidx_path)?;
                    let mut blobs = 0;
                    let mut trees = 0;
                    let mut commits = 0;
                    let mut tags = 0;
                    let mut uncompressed_total = 0u64;

                    for oid in idx_reader.object_ids() {
                        if let Some(entry) = idx_reader.lookup(oid) {
                            uncompressed_total += entry.orig_len as u64;
                            let (_, obj_type, _) = reader.read_object_at(entry.offset)?;
                            match obj_type {
                                ObjectType::Blob => blobs += 1,
                                ObjectType::Tree => trees += 1,
                                ObjectType::Commit => commits += 1,
                                ObjectType::Tag => tags += 1,
                            }
                        }
                    }
                    let pack_size = fs::metadata(&spk_path)?.len();
                    let ratio = if uncompressed_total > 0 {
                        (1.0 - (pack_size as f64 / uncompressed_total as f64)) * 100.0
                    } else {
                        0.0
                    };
                    println!("Packfile: {}", id);
                    println!("  Total Objects:        {}", reader.object_count());
                    println!("    Blobs:              {}", blobs);
                    println!("    Trees:              {}", trees);
                    println!("    Commits:            {}", commits);
                    println!("    Tags:               {}", tags);
                    println!(
                        "  Pack Size:            {} ({} bytes)",
                        format_bytes(pack_size),
                        pack_size
                    );
                    println!(
                        "  Uncompressed Size:    {} ({} bytes)",
                        format_bytes(uncompressed_total),
                        uncompressed_total
                    );
                    println!("  Compression Savings:  {:.1}%", ratio.max(0.0));
                }
            }
        }

        Commands::Repack { dry_run, verify } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;
            let _lock = RepoLock::acquire(&spine_dir, "repack")?;

            println!("Starting repository repack...");
            let summary = repack_loose_objects(&store, 3, dry_run)?;
            if dry_run {
                println!(
                    "Dry-run: {} loose objects would be consolidated into a new pack.",
                    summary.objects_repacked
                );
                println!(
                    "Total loose bytes to repack: {}",
                    format_bytes(summary.original_bytes)
                );
            } else if summary.objects_repacked == 0 {
                println!("Nothing to repack: 0 loose objects found.");
            } else {
                println!("Repack complete:");
                println!(
                    "  Packfile:          {}",
                    summary.pack_id.as_deref().unwrap_or("none")
                );
                println!("  Objects repacked:  {}", summary.objects_repacked);
                println!(
                    "  Original size:     {}",
                    format_bytes(summary.original_bytes)
                );
                println!(
                    "  Packfile size:     {}",
                    format_bytes(summary.packed_bytes)
                );
                println!("  Index size:        {}", format_bytes(summary.index_bytes));
                println!("  Loose pruned:      {}", summary.loose_deleted);
                if verify {
                    println!("Verifying repacked storage integrity...");
                    let audit = store.verify_storage()?;
                    if audit.corrupted_objects.is_empty() && audit.corrupted_packs.is_empty() {
                        println!(
                            "Integrity check PASSED: {} objects verified across all packs.",
                            audit.total_verified_objects
                        );
                    } else {
                        bail!("Integrity check FAILED: corrupt objects or packs detected!");
                    }
                }
            }
        }

        Commands::Gc {
            dry_run,
            prune_before,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let _lock = RepoLock::acquire(&ctx.primary_spine_dir, "gc")?;

            // Comprehensive enumeration of all GC roots across repository:
            // 1. Branch tips
            let mut roots = Vec::new();
            for (_, oid_opt) in list_branches(&ctx.primary_spine_dir)? {
                if let Some(oid) = oid_opt {
                    roots.push(oid);
                }
            }

            // 2. All tags (annotated and lightweight)
            let tags_dir = ctx.primary_spine_dir.join("refs").join("tags");
            if tags_dir.exists() {
                for entry in fs::read_dir(tags_dir)?.flatten() {
                    if let Ok(c) = fs::read_to_string(entry.path()) {
                        if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                            roots.push(oid);
                        }
                    }
                }
            }

            // 3. Remote tracking refs
            let remotes_dir = ctx.primary_spine_dir.join("refs").join("remotes");
            if remotes_dir.exists() {
                for entry in fs::read_dir(remotes_dir)?.flatten() {
                    if entry.path().is_dir() {
                        for sub in fs::read_dir(entry.path())?.flatten() {
                            if let Ok(c) = fs::read_to_string(sub.path()) {
                                if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                                    roots.push(oid);
                                }
                            }
                        }
                    } else if let Ok(c) = fs::read_to_string(entry.path()) {
                        if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                            roots.push(oid);
                        }
                    }
                }
            }

            // 4. Primary HEAD and current worktree HEAD
            for s_dir in &[&ctx.primary_spine_dir, &ctx.spine_dir] {
                if let Ok(state) = resolve_head(s_dir) {
                    if let Some(cid) = state.head_commit_id {
                        roots.push(cid);
                    }
                }
            }

            // 5. Stash stack entries and refs/stash
            let refs_stash = ctx.primary_spine_dir.join("refs").join("stash");
            if refs_stash.exists() {
                if let Ok(c) = fs::read_to_string(refs_stash) {
                    if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                        roots.push(oid);
                    }
                }
            }
            if let Ok(stash_list) = spine_worktree::StashList::load_from_dir(&ctx.primary_spine_dir)
            {
                for entry in stash_list.entries {
                    roots.push(entry.commit_id);
                    roots.push(entry.worktree_tree);
                    roots.push(entry.index_tree);
                    if let Some(utree) = entry.untracked_tree {
                        roots.push(utree);
                    }
                }
            }

            // 6. Linked worktrees (HEADs and indexes)
            if let Ok(worktrees) = spine_worktree::list_worktrees(&ctx.primary_spine_dir) {
                for wt in worktrees {
                    if let Some(h) = wt.head_oid {
                        roots.push(h);
                    }
                    let wt_idx = ctx
                        .primary_spine_dir
                        .join("worktrees")
                        .join(&wt.id)
                        .join("index");
                    if let Ok(idx) = Index::load_from_file(&wt_idx) {
                        for entry in idx.iter() {
                            roots.push(entry.object_id);
                        }
                    }
                }
            }

            // 7. In-flight operation states
            if let Ok(Some(ms)) = MergeState::load_from_dir(&ctx.spine_dir) {
                roots.push(ms.ours_commit);
                roots.push(ms.theirs_commit);
                roots.push(ms.base_commit);
            }
            if let Ok(Some(rs)) = spine_merge::RebaseState::load_from_dir(&ctx.spine_dir) {
                roots.push(rs.onto);
                roots.push(rs.orig_head);
                if let Some(c) = rs.current_commit {
                    roots.push(c);
                }
                for r in rs.remaining_commits {
                    roots.push(r);
                }
                for (_, new_oid) in rs.old_to_new {
                    roots.push(new_oid);
                }
            }
            if let Ok(Some(cps)) = spine_merge::CherryPickState::load_from_dir(&ctx.spine_dir) {
                roots.push(cps.target_commit);
                roots.push(cps.orig_head);
            }
            if let Ok(Some(rvs)) = spine_merge::RevertState::load_from_dir(&ctx.spine_dir) {
                roots.push(rvs.target_commit);
                roots.push(rvs.orig_head);
            }
            if let Ok(Some(bs)) = spine_history::BisectState::load_from_dir(&ctx.spine_dir) {
                roots.push(bs.bad_commit);
                roots.push(bs.orig_head);
                for g in bs.good_commits {
                    roots.push(g);
                }
                for s in bs.skipped_commits {
                    roots.push(s);
                }
                if let Some(curr) = bs.current_commit {
                    roots.push(curr);
                }
            }

            // 8. Staging index
            let index_path = ctx.spine_dir.join("index");
            if let Ok(idx) = Index::load_from_file(&index_path) {
                for entry in idx.iter() {
                    roots.push(entry.object_id);
                }
            }

            // 9. Operation journal records
            if let Ok(journal) = Journal::new(ctx.spine_dir.join("journal")) {
                if let Ok(records) = journal.list_records() {
                    for r in records {
                        if let Some(tip) = r.before_state.branch_tip {
                            roots.push(tip);
                        }
                        if let Some(tree) = r.before_state.tree_id {
                            roots.push(tree);
                        }
                        if let Some(ref after) = r.after_state {
                            if let Some(tip) = after.branch_tip {
                                roots.push(tip);
                            }
                            if let Some(tree) = after.tree_id {
                                roots.push(tree);
                            }
                        }
                    }
                }
            }

            let duration = parse_duration(&prune_before)
                .unwrap_or(Duration::from_secs(DEFAULT_GRACE_PERIOD_SECS));
            println!(
                "Executing garbage collection (grace period: {:?})...",
                duration
            );
            let gc_summary = collect_garbage(&store, &roots, duration, dry_run)?;

            if dry_run {
                println!(
                    "Dry-run: {} unreachable objects eligible for pruning ({}).",
                    gc_summary.pruned_objects,
                    format_bytes(gc_summary.bytes_reclaimed)
                );
                println!(
                    "Preserved by grace period: {} objects.",
                    gc_summary.grace_period_preserved
                );
            } else {
                println!("Garbage collection complete:");
                println!(
                    "  Objects inspected:      {}",
                    gc_summary.total_objects_inspected
                );
                println!("  Reachable objects:      {}", gc_summary.reachable_objects);
                println!(
                    "  Grace-period preserved: {}",
                    gc_summary.grace_period_preserved
                );
                println!("  Pruned objects:         {}", gc_summary.pruned_objects);
                println!(
                    "  Reclaimed disk space:   {}",
                    format_bytes(gc_summary.bytes_reclaimed)
                );
            }
        }

        Commands::VerifyStorage => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config = load_repo_config(&spine_dir)?;
            let store = open_store(&spine_dir, &config)?;

            println!("Verifying Storage Engine V2 repository integrity...");
            let report = store.verify_storage()?;
            println!("  Packs audited:          {}", report.total_packs_audited);
            println!("  Loose objects audited:  {}", report.total_loose_audited);
            println!(
                "  Total verified objects: {}",
                report.total_verified_objects
            );

            if report.corrupted_objects.is_empty() && report.corrupted_packs.is_empty() {
                println!("All storage objects and packs are cryptographically healthy.");
            } else {
                if !report.corrupted_packs.is_empty() {
                    eprintln!("Corrupted pack files: {:?}", report.corrupted_packs);
                }
                if !report.corrupted_objects.is_empty() {
                    eprintln!("Corrupted objects: {:?}", report.corrupted_objects);
                }
                bail!("Storage verification detected corruption!");
            }
        }

        Commands::Remote { action } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config_path = spine_dir.join("spine.toml");
            let mut config = SpineConfig::load_from_file(&config_path)?;

            match action.unwrap_or(RemoteAction::List) {
                RemoteAction::Add { name, url } => {
                    SpineUrl::parse(&url).context("Invalid remote URL")?;
                    config.add_remote(&name, &url);
                    config.save_to_file(&config_path)?;
                    println!("Added remote '{}' with URL '{}'", name, url);
                }
                RemoteAction::List => {
                    if config.remote.is_empty() {
                        println!("No remotes configured.");
                    } else {
                        for (name, r) in &config.remote {
                            println!("{}\t{}", name, r.url);
                        }
                    }
                }
                RemoteAction::Remove { name } => {
                    if config.remove_remote(&name) {
                        config.save_to_file(&config_path)?;
                        let tracking_dir = spine_dir.join("refs").join("remotes").join(&name);
                        let _ = fs::remove_dir_all(tracking_dir);
                        println!("Removed remote '{}'", name);
                    } else {
                        bail!("error: No such remote: '{}'", name);
                    }
                }
                RemoteAction::GetUrl { name } => {
                    if let Some(r) = config.get_remote(&name) {
                        println!("{}", r.url);
                    } else {
                        bail!("error: No such remote: '{}'", name);
                    }
                }
                RemoteAction::SetUrl { name, url } => {
                    SpineUrl::parse(&url).context("Invalid remote URL")?;
                    if config.set_remote_url(&name, &url) {
                        config.save_to_file(&config_path)?;
                        println!("Updated remote '{}' URL to '{}'", name, url);
                    } else {
                        bail!("error: No such remote: '{}'", name);
                    }
                }
            }
        }

        Commands::Clone { url, directory } => {
            let parsed_url = SpineUrl::parse(&url).context("Invalid clone URL")?;
            let target_dir = match directory {
                Some(d) => {
                    if d.is_absolute() {
                        d
                    } else {
                        env::current_dir()?.join(d)
                    }
                }
                None => {
                    let name = parsed_url
                        .path
                        .split('/')
                        .next_back()
                        .unwrap_or("repository");
                    env::current_dir()?.join(name)
                }
            };

            if target_dir.exists() {
                let is_empty = fs::read_dir(&target_dir)?.next().is_none();
                if !is_empty {
                    bail!(
                        "fatal: destination path '{}' already exists and is not an empty directory.",
                        target_dir.display()
                    );
                }
            } else {
                fs::create_dir_all(&target_dir)?;
            }

            println!("Cloning into '{}'...", target_dir.display());

            // 1. Initialize repository structure
            let spine_dir = target_dir.join(".spine");
            fs::create_dir_all(spine_dir.join("objects").join("pack"))?;
            fs::create_dir_all(spine_dir.join("refs").join("heads"))?;
            fs::create_dir_all(spine_dir.join("refs").join("tags"))?;
            fs::create_dir_all(spine_dir.join("refs").join("remotes").join("origin"))?;

            // 2. Write initial config with origin
            let config_path = spine_dir.join("spine.toml");
            let mut config = SpineConfig::default();
            config.add_remote("origin", &url);
            config.save_to_file(&config_path)?;

            let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3)?;

            // 3. Connect to remote
            let mut client = SpineClient::connect(&url, None, None)
                .await
                .context("Failed to connect to Spine daemon")?;

            let default_branch = client
                .repo_info()
                .map(|i| i.default_branch.clone())
                .unwrap_or_else(|| "main".to_string());

            let remote_ref_name = format!("refs/heads/{}", default_branch);
            let advertised = client.advertised_refs().clone();

            let target_oid = match advertised.get(&remote_ref_name) {
                Some(oid) => *oid,
                None => {
                    println!("warning: You appear to have cloned an empty repository.");
                    fs::write(
                        spine_dir.join("HEAD"),
                        format!("ref: refs/heads/{}\n", default_branch),
                    )?;
                    return Ok(());
                }
            };

            // 4. Fetch missing objects
            let _ = client
                .fetch(&[target_oid], &[], &spine_dir, &store)
                .await
                .context("Failed to stream pack during clone")?;

            // 5. Update local references
            fs::write(
                spine_dir.join("refs").join("heads").join(&default_branch),
                format!("{}\n", target_oid.to_hex()),
            )?;
            fs::write(
                spine_dir
                    .join("refs")
                    .join("remotes")
                    .join("origin")
                    .join(&default_branch),
                format!("{}\n", target_oid.to_hex()),
            )?;
            fs::write(
                spine_dir.join("HEAD"),
                format!("ref: refs/heads/{}\n", default_branch),
            )?;

            // 6. Check out working tree
            let commit_record = get_commit(&store, &target_oid)?;
            let mut new_index = Index::new();

            execute_switch(
                &target_dir,
                &mut new_index,
                &store,
                None,
                &commit_record.root_tree,
            )?;
            new_index.save_to_file(spine_dir.join("index"))?;

            println!("Checking out files: done.");
            println!("Switched to a new branch '{}'", default_branch);
        }

        Commands::Fetch { remote, all } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config_path = spine_dir.join("spine.toml");
            let config = SpineConfig::load_from_file(&config_path)?;
            let store = open_store(&spine_dir, &config)?;

            let remotes_to_fetch: Vec<(&str, &str)> = if all {
                config
                    .remote
                    .iter()
                    .map(|(k, v)| (k.as_str(), v.url.as_str()))
                    .collect()
            } else {
                let r = config.get_remote(&remote).ok_or_else(|| {
                    anyhow::anyhow!(
                        "fatal: '{}' does not appear to be a spine repository",
                        remote
                    )
                })?;
                vec![(remote.as_str(), r.url.as_str())]
            };

            for (r_name, r_url) in remotes_to_fetch {
                println!("Fetching from {} ({})", r_name, r_url);
                let mut client = SpineClient::connect(r_url, None, None)
                    .await
                    .context(format!("Failed to connect to remote '{}'", r_name))?;

                let advertised = client.advertised_refs().clone();

                let mut have = Vec::new();
                let heads_dir = spine_dir.join("refs").join("heads");
                if heads_dir.exists() {
                    for entry in fs::read_dir(&heads_dir)?.flatten() {
                        if let Ok(c) = fs::read_to_string(entry.path()) {
                            if let Ok(oid) = ObjectId::from_hex(c.trim()) {
                                have.push(oid);
                            }
                        }
                    }
                }

                let mut want = Vec::new();
                let mut updates = Vec::new();

                for (ref_name, remote_oid) in &advertised {
                    if let Some(branch) = ref_name.strip_prefix("refs/heads/") {
                        let tracking_file = spine_dir
                            .join("refs")
                            .join("remotes")
                            .join(r_name)
                            .join(branch);

                        let current_tracking = if tracking_file.exists() {
                            let c = fs::read_to_string(&tracking_file).unwrap_or_default();
                            ObjectId::from_hex(c.trim()).ok()
                        } else {
                            None
                        };

                        if current_tracking != Some(*remote_oid) {
                            want.push(*remote_oid);
                            updates.push((branch.to_string(), current_tracking, *remote_oid));
                        }
                    }
                }

                if want.is_empty() {
                    println!("From {}", r_url);
                    println!(" = [up to date]      all branches match remote");
                    continue;
                }

                let _ = client.fetch(&want, &have, &spine_dir, &store).await?;

                println!("From {}", r_url);
                for (branch, old_oid, new_oid) in updates {
                    let tracking_file = spine_dir
                        .join("refs")
                        .join("remotes")
                        .join(r_name)
                        .join(&branch);
                    if let Some(parent) = tracking_file.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&tracking_file, format!("{}\n", new_oid.to_hex()))?;

                    let old_short = old_oid
                        .map(|o| o.to_hex()[..7].to_string())
                        .unwrap_or_else(|| "[new]".to_string());
                    let new_short = &new_oid.to_hex()[..7];
                    println!(
                        "   {}..{}  {} -> {}/{}",
                        old_short, new_short, branch, r_name, branch
                    );
                }
            }
        }

        Commands::Push {
            remote,
            branch,
            force,
            force_with_lease,
        } => {
            let (_worktree_root, spine_dir) = find_repo_root()?;
            let config_path = spine_dir.join("spine.toml");
            let config = SpineConfig::load_from_file(&config_path)?;
            let store = open_store(&spine_dir, &config)?;

            let r = config.get_remote(&remote).ok_or_else(|| {
                anyhow::anyhow!(
                    "fatal: '{}' does not appear to be a spine repository",
                    remote
                )
            })?;

            let target_branch = match branch {
                Some(b) => b,
                None => {
                    let head_content = fs::read_to_string(spine_dir.join("HEAD"))?;
                    if let Some(rest) = head_content.strip_prefix("ref: refs/heads/") {
                        rest.trim().to_string()
                    } else {
                        bail!("fatal: You are not currently on a branch. Please specify branch to push.");
                    }
                }
            };

            let local_ref_file = spine_dir.join("refs").join("heads").join(&target_branch);
            if !local_ref_file.exists() {
                bail!("fatal: Local branch '{}' does not exist.", target_branch);
            }
            let local_oid_str = fs::read_to_string(&local_ref_file)?;
            let local_oid = ObjectId::from_hex(local_oid_str.trim())?;

            println!("Pushing to {} ({})", remote, r.url);
            let mut client = SpineClient::connect(&r.url, None, None)
                .await
                .context(format!("Failed to connect to remote '{}'", remote))?;

            let remote_ref_name = format!("refs/heads/{}", target_branch);
            let advertised = client.advertised_refs().clone();
            let remote_oid = advertised.get(&remote_ref_name).copied();

            if remote_oid == Some(local_oid) {
                println!("Everything up-to-date");
                return Ok(());
            }

            let tracking_file = spine_dir
                .join("refs")
                .join("remotes")
                .join(&remote)
                .join(&target_branch);

            let tracking_oid = if tracking_file.exists() {
                fs::read_to_string(&tracking_file)
                    .ok()
                    .and_then(|s| ObjectId::from_hex(s.trim()).ok())
            } else {
                None
            };

            let (effective_force, lease_oid) = if let Some(lease_spec) = force_with_lease {
                let lease = if lease_spec.is_empty() {
                    tracking_oid.or(remote_oid)
                } else {
                    Some(ObjectId::from_hex(&lease_spec)?)
                };
                (true, lease)
            } else {
                (force, None)
            };

            let proposal = RefUpdateProposal {
                ref_name: remote_ref_name.clone(),
                expected_old_oid: remote_oid,
                new_oid: local_oid,
                force: effective_force,
                lease_oid,
            };

            let remote_tips: Vec<ObjectId> = advertised.values().copied().collect();
            let res = client
                .push(&[proposal], &store, &remote_tips)
                .await
                .context("Push operation failed")?;

            for update_res in res.results {
                if update_res.status == "ok" {
                    if let Some(parent) = tracking_file.parent() {
                        fs::create_dir_all(parent)?;
                    }
                    fs::write(&tracking_file, format!("{}\n", local_oid.to_hex()))?;

                    let old_short = remote_oid
                        .map(|o| o.to_hex()[..7].to_string())
                        .unwrap_or_else(|| "[new branch]".to_string());
                    let new_short = &local_oid.to_hex()[..7];
                    println!("To {}", r.url);
                    println!(
                        "   {}..{}  {} -> {}",
                        old_short, new_short, target_branch, target_branch
                    );
                } else if update_res.status == "non_fast_forward" {
                    eprintln!("To {}", r.url);
                    eprintln!(
                        " ! [rejected]        {} -> {} (non-fast-forward)",
                        target_branch, target_branch
                    );
                    eprintln!("error: failed to push some refs to '{}'", r.url);
                    eprintln!("hint: Updates were rejected because the tip of your current branch is behind");
                    eprintln!("hint: its remote counterpart. Integrate the remote changes (e.g.");
                    eprintln!(
                        "hint: 'spine pull {} {}') before pushing again.",
                        remote, target_branch
                    );
                    eprintln!("hint: See 'spine push --help' for options like --force or --force-with-lease.");
                    bail!("Push rejected: non-fast-forward");
                } else if update_res.status == "stale_ref" {
                    eprintln!("To {}", r.url);
                    eprintln!(
                        " ! [rejected]        {} -> {} (stale reference or lease mismatch)",
                        target_branch, target_branch
                    );
                    if let Some(msg) = update_res.message {
                        eprintln!("error: {}", msg);
                    } else {
                        eprintln!("error: Remote reference changed concurrently by another user.");
                    }
                    bail!("Push rejected: stale reference");
                } else if update_res.status == "permission_denied" {
                    eprintln!("To {}", r.url);
                    eprintln!(
                        " ! [rejected]        {} -> {} (permission denied)",
                        target_branch, target_branch
                    );
                    if let Some(msg) = update_res.message {
                        eprintln!("error: {}", msg);
                    }
                    bail!("Push rejected: permission denied");
                } else if update_res.status == "missing_object" {
                    eprintln!("To {}", r.url);
                    eprintln!(
                        " ! [rejected]        {} -> {} (object closure incomplete)",
                        target_branch, target_branch
                    );
                    if let Some(msg) = update_res.message {
                        eprintln!("error: {}", msg);
                    }
                    bail!("Push rejected: missing object in packfile");
                } else {
                    bail!("Push failed: {}", update_res.status);
                }
            }
        }

        Commands::Pull { remote, branch } => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config_path = spine_dir.join("spine.toml");
            let config = SpineConfig::load_from_file(&config_path)?;
            let store = open_store(&spine_dir, &config)?;

            let target_branch = match branch {
                Some(b) => b,
                None => {
                    let head_content = fs::read_to_string(spine_dir.join("HEAD"))?;
                    if let Some(rest) = head_content.strip_prefix("ref: refs/heads/") {
                        rest.trim().to_string()
                    } else {
                        bail!("fatal: You are not currently on a branch.");
                    }
                }
            };

            let r = config.get_remote(&remote).ok_or_else(|| {
                anyhow::anyhow!(
                    "fatal: '{}' does not appear to be a spine repository",
                    remote
                )
            })?;

            let mut client = SpineClient::connect(&r.url, None, None)
                .await
                .context(format!("Failed to connect to remote '{}'", remote))?;

            let remote_ref_name = format!("refs/heads/{}", target_branch);
            let advertised = client.advertised_refs().clone();

            let remote_oid = match advertised.get(&remote_ref_name) {
                Some(oid) => *oid,
                None => {
                    println!(
                        "Your branch is up to date (remote has no branch '{}').",
                        target_branch
                    );
                    return Ok(());
                }
            };

            let local_ref_file = spine_dir.join("refs").join("heads").join(&target_branch);
            let local_oid = if local_ref_file.exists() {
                let s = fs::read_to_string(&local_ref_file)?;
                ObjectId::from_hex(s.trim())?
            } else {
                bail!("Local branch '{}' does not exist.", target_branch);
            };

            if local_oid == remote_oid {
                println!("Already up to date.");
                return Ok(());
            }

            let _ = client
                .fetch(&[remote_oid], &[local_oid], &spine_dir, &store)
                .await?;

            let tracking_file = spine_dir
                .join("refs")
                .join("remotes")
                .join(&remote)
                .join(&target_branch);
            if let Some(parent) = tracking_file.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(&tracking_file, format!("{}\n", remote_oid.to_hex()))?;

            let can_ff = is_ancestor(&store, &local_oid, &remote_oid)?;
            if can_ff {
                fs::write(&local_ref_file, format!("{}\n", remote_oid.to_hex()))?;

                let commit_record = get_commit(&store, &remote_oid)?;
                let mut index =
                    Index::load_from_file(spine_dir.join("index")).unwrap_or_else(|_| Index::new());

                execute_switch(
                    &worktree_root,
                    &mut index,
                    &store,
                    Some(&get_commit(&store, &local_oid)?.root_tree),
                    &commit_record.root_tree,
                )?;
                index.save_to_file(spine_dir.join("index"))?;

                println!(
                    "Updating {}..{}",
                    &local_oid.to_hex()[..7],
                    &remote_oid.to_hex()[..7]
                );
                println!("Fast-forward complete.");
            } else {
                eprintln!("warning: Divergent branches detected!");
                eprintln!(
                    "Local tip {} and remote tip {} have diverged.",
                    &local_oid.to_hex()[..7],
                    &remote_oid.to_hex()[..7]
                );
                eprintln!(
                    "Run 'spine merge {}/{}' to merge changes.",
                    remote, target_branch
                );
                bail!("Automatic fast-forward not possible; manual merge required.");
            }
        }

        Commands::Tag { action } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;

            match action.unwrap_or(TagAction::List) {
                TagAction::List => {
                    ops_tag::cmd_tag_list(&ctx.primary_spine_dir)?;
                }
                TagAction::Create {
                    name,
                    rev,
                    annotate,
                    message,
                } => {
                    let target_oid = resolve_commit_ref_advanced(
                        &ctx.spine_dir,
                        &ctx.primary_spine_dir,
                        &store,
                        &rev,
                    )?;
                    ops_tag::cmd_tag_create(
                        &ctx.primary_spine_dir,
                        &store,
                        &config,
                        &name,
                        target_oid,
                        annotate,
                        message,
                    )?;
                }
                TagAction::Show { name } => {
                    ops_tag::cmd_tag_show(&ctx.primary_spine_dir, &store, &name)?;
                }
                TagAction::Verify { name } => {
                    ops_tag::cmd_tag_verify(&ctx.primary_spine_dir, &store, &name)?;
                }
                TagAction::Delete { name } => {
                    ops_tag::cmd_tag_delete(&ctx.primary_spine_dir, &name)?;
                }
            }
        }

        Commands::Reset {
            soft,
            mixed,
            hard,
            revision,
            paths,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            let ignore = IgnoreRules::load_from_dir(&ctx.worktree_root);
            let target_oid = resolve_commit_ref_advanced(
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &store,
                &revision,
            )?;

            ops_history::cmd_reset(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &mut index,
                &store,
                &target_oid,
                soft,
                mixed,
                hard,
                &paths,
                &ignore,
            )?;
        }

        Commands::Restore {
            staged,
            worktree,
            source,
            paths,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            let source_oid = match source {
                Some(ref rev) => Some(resolve_commit_ref_advanced(
                    &ctx.spine_dir,
                    &ctx.primary_spine_dir,
                    &store,
                    rev,
                )?),
                None => {
                    if staged {
                        resolve_head(&ctx.spine_dir)?.head_commit_id
                    } else {
                        None
                    }
                }
            };

            ops_history::cmd_restore(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &mut index,
                &store,
                &paths,
                staged,
                worktree,
                source_oid,
            )?;
        }

        Commands::Revert {
            revision,
            mainline,
            continue_revert,
            abort,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;

            let target_oid = match revision {
                Some(ref r) => Some(resolve_commit_ref_advanced(
                    &ctx.spine_dir,
                    &ctx.primary_spine_dir,
                    &store,
                    r,
                )?),
                None => None,
            };

            ops_history::cmd_revert(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &mut index,
                &store,
                target_oid,
                mainline,
                continue_revert,
                abort,
                config.user.name.as_deref().unwrap_or("Spine Developer"),
                config.user.email.as_deref().unwrap_or("dev@spine.org"),
            )?;
        }

        Commands::CherryPick {
            revision,
            mainline,
            continue_cherry_pick,
            skip,
            abort,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;

            let target_oid = match revision {
                Some(ref r) => Some(resolve_commit_ref_advanced(
                    &ctx.spine_dir,
                    &ctx.primary_spine_dir,
                    &store,
                    r,
                )?),
                None => None,
            };

            ops_history::cmd_cherry_pick(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &mut index,
                &store,
                target_oid,
                mainline,
                continue_cherry_pick,
                skip,
                abort,
            )?;
        }

        Commands::Rebase {
            upstream,
            onto,
            continue_rebase,
            skip,
            abort,
        } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;

            let upstream_oid = match upstream {
                Some(ref u) => Some(resolve_commit_ref_advanced(
                    &ctx.spine_dir,
                    &ctx.primary_spine_dir,
                    &store,
                    u,
                )?),
                None => None,
            };

            let onto_oid = match onto {
                Some(ref o) => Some(resolve_commit_ref_advanced(
                    &ctx.spine_dir,
                    &ctx.primary_spine_dir,
                    &store,
                    o,
                )?),
                None => None,
            };

            ops_history::cmd_rebase(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &mut index,
                &store,
                upstream_oid,
                onto_oid,
                continue_rebase,
                skip,
                abort,
            )?;
        }

        Commands::Stash { action } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            let ignore = IgnoreRules::load_from_dir(&ctx.worktree_root);
            let state = resolve_head(&ctx.spine_dir)?;

            match action.unwrap_or(StashAction::Push {
                message: None,
                include_untracked: false,
            }) {
                StashAction::Push {
                    message,
                    include_untracked,
                } => {
                    let branch_name = state.head_ref_name.unwrap_or_else(|| "HEAD".into());
                    ops_worktree::cmd_stash_push(
                        &ctx.worktree_root,
                        &ctx.spine_dir,
                        &ctx.primary_spine_dir,
                        &mut index,
                        &store,
                        state.head_commit_id.as_ref(),
                        &branch_name,
                        message.as_deref(),
                        include_untracked,
                        &ignore,
                    )?;
                }
                StashAction::List => {
                    ops_worktree::cmd_stash_list(&ctx.primary_spine_dir)?;
                }
                StashAction::Show { index: idx } => {
                    ops_worktree::cmd_stash_show(&ctx.primary_spine_dir, &store, idx)?;
                }
                StashAction::Apply {
                    index: idx,
                    index_reinstate,
                } => {
                    ops_worktree::cmd_stash_apply(
                        &ctx.worktree_root,
                        &ctx.spine_dir,
                        &ctx.primary_spine_dir,
                        &mut index,
                        &store,
                        idx,
                        index_reinstate,
                    )?;
                }
                StashAction::Pop {
                    index: idx,
                    index_reinstate,
                } => {
                    ops_worktree::cmd_stash_pop(
                        &ctx.worktree_root,
                        &ctx.spine_dir,
                        &ctx.primary_spine_dir,
                        &mut index,
                        &store,
                        idx,
                        index_reinstate,
                    )?;
                }
                StashAction::Drop { index: idx } => {
                    ops_worktree::cmd_stash_drop(&ctx.primary_spine_dir, idx)?;
                }
                StashAction::Clear => {
                    ops_worktree::cmd_stash_clear(&ctx.primary_spine_dir)?;
                }
            }
        }

        Commands::Worktree { action } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;

            match action {
                WorktreeAction::Add { path, branch } => {
                    let head_state = resolve_head(&ctx.spine_dir)?;
                    let target_oid = if let Some(ref b) = branch {
                        if let Ok(oid) = resolve_commit_ref_advanced(
                            &ctx.spine_dir,
                            &ctx.primary_spine_dir,
                            &store,
                            b,
                        ) {
                            oid
                        } else {
                            head_state
                                .head_commit_id
                                .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?
                        }
                    } else {
                        head_state
                            .head_commit_id
                            .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"))?
                    };

                    ops_worktree::cmd_worktree_add(
                        &ctx.primary_spine_dir,
                        &store,
                        &path,
                        branch,
                        &target_oid,
                    )?;
                }
                WorktreeAction::List => {
                    ops_worktree::cmd_worktree_list(&ctx.primary_spine_dir)?;
                }
                WorktreeAction::Remove { id_or_path, force } => {
                    ops_worktree::cmd_worktree_remove(&ctx.primary_spine_dir, &id_or_path, force)?;
                }
                WorktreeAction::Prune => {
                    ops_worktree::cmd_worktree_prune(&ctx.primary_spine_dir)?;
                }
            }
        }

        Commands::Blame { path, revision } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let target_oid = resolve_commit_ref_advanced(
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &store,
                &revision,
            )?;
            ops_history::cmd_blame(&store, &target_oid, &path)?;
        }

        Commands::Bisect { action } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;

            ops_history::cmd_bisect(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &ctx.primary_spine_dir,
                &mut index,
                &store,
                &action,
            )?;
        }

        Commands::Mv {
            source,
            destination,
        } => {
            let ctx = get_repo_context()?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            ops_worktree::cmd_mv(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &mut index,
                &source,
                &destination,
            )?;
        }

        Commands::Rm {
            paths,
            cached,
            force,
        } => {
            let ctx = get_repo_context()?;
            let mut index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            ops_worktree::cmd_rm(
                &ctx.worktree_root,
                &ctx.spine_dir,
                &mut index,
                &paths,
                cached,
                force,
            )?;
        }

        Commands::Clean { dry_run, force } => {
            let ctx = get_repo_context()?;
            let config = SpineConfig::load_from_file(ctx.primary_spine_dir.join("spine.toml"))?;
            let store = open_store(&ctx.primary_spine_dir, &config)?;
            let index = Index::load_from_file(ctx.spine_dir.join("index"))?;
            let ignore = IgnoreRules::load_from_dir(&ctx.worktree_root);
            let state = resolve_head(&ctx.spine_dir)?;
            let head_tree_id = match state.head_commit_id {
                Some(cid) => get_commit(&store, &cid).ok().map(|c| c.root_tree),
                None => None,
            };

            ops_worktree::cmd_clean(
                &ctx.worktree_root,
                &index,
                &store,
                head_tree_id.as_ref(),
                dry_run,
                force,
                &ignore,
            )?;
        }
    }

    Ok(())
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim().to_lowercase();
    if s == "0" || s == "0s" || s == "none" {
        return Some(Duration::ZERO);
    }
    if let Some(rest) = s.strip_suffix('d') {
        let val: u64 = rest.parse().ok()?;
        return Some(Duration::from_secs(val * 86400));
    }
    if let Some(rest) = s.strip_suffix('h') {
        let val: u64 = rest.parse().ok()?;
        return Some(Duration::from_secs(val * 3600));
    }
    if let Some(rest) = s.strip_suffix('m') {
        let val: u64 = rest.parse().ok()?;
        return Some(Duration::from_secs(val * 60));
    }
    if let Some(rest) = s.strip_suffix('s') {
        let val: u64 = rest.parse().ok()?;
        return Some(Duration::from_secs(val));
    }
    s.parse::<u64>().ok().map(Duration::from_secs)
}
