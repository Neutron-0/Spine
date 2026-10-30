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
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use spine_worktree::{
    check_switch_safety, compute_status, execute_switch, flatten_tree, metadata_to_stat,
    stage_path, write_index_to_tree, IgnoreRules,
};
use std::collections::BTreeMap;
use std::env;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
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

fn find_repo_root() -> Result<(PathBuf, PathBuf)> {
    let mut current = env::current_dir().context("Failed to get current working directory")?;
    loop {
        let spine_dir = current.join(".spine");
        if spine_dir.is_dir() && spine_dir.join("spine.toml").is_file() {
            return Ok((current, spine_dir));
        }
        if !current.pop() {
            bail!("fatal: not a spine repository (or any of the parent directories): .spine");
        }
    }
}

fn open_store(spine_dir: &Path, config: &SpineConfig) -> Result<FileSystemObjectStore> {
    let algo = match config.repository.hash_algorithm.as_str() {
        "sha256" => HashAlgorithm::Sha256,
        _ => HashAlgorithm::Blake3,
    };
    let store =
        FileSystemObjectStore::new(spine_dir, algo, 3).context("Failed to open object store")?;
    Ok(store)
}

#[derive(Clone, Debug)]
struct RepoState {
    head_ref_name: Option<String>,
    head_commit_id: Option<ObjectId>,
}

fn resolve_head(spine_dir: &Path) -> Result<RepoState> {
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

fn resolve_commit_ref(
    spine_dir: &Path,
    store: &FileSystemObjectStore,
    rev: &str,
) -> Result<ObjectId> {
    if rev == "HEAD" {
        let state = resolve_head(spine_dir)?;
        return state
            .head_commit_id
            .ok_or_else(|| anyhow::anyhow!("HEAD has no commits yet"));
    }

    // Check branch
    let branch_ref = spine_dir.join("refs").join("heads").join(rev);
    if branch_ref.exists() {
        let content = fs::read_to_string(branch_ref)?.trim().to_string();
        return ObjectId::from_hex(&content).context("Invalid branch ref");
    }

    // Direct hash
    if let Ok(oid) = ObjectId::from_hex(rev) {
        if store.has_object(&oid) {
            return Ok(oid);
        }
    }

    bail!("fatal: Cannot resolve revision '{}'", rev)
}

fn update_ref(spine_dir: &Path, target_ref: &str, new_oid: &ObjectId) -> Result<()> {
    let ref_path = spine_dir.join(target_ref);
    let parent = ref_path.parent().unwrap();
    fs::create_dir_all(parent)?;

    let tmp_path = spine_dir
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

fn capture_snapshot(
    spine_dir: &Path,
    state: &RepoState,
    store: &FileSystemObjectStore,
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
    })
}

fn restore_snapshot(
    worktree_root: &Path,
    spine_dir: &Path,
    store: &FileSystemObjectStore,
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

fn summarize_diff<S: ObjectStore>(
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

fn main() -> Result<()> {
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
            let store = open_store(&spine_dir, &config)?;
            let index_path = spine_dir.join("index");
            let mut index = Index::load_from_file(&index_path)?;
            let ignore = IgnoreRules::load_from_dir(&worktree_root);

            let mut total_staged = 0;
            for p in paths {
                total_staged += stage_path(&worktree_root, &p, &mut index, &store, &ignore)?;
            }

            index.save_to_file(&index_path)?;
            if total_staged == 0 {
                println!("No changes staged.");
            }
        }

        Commands::Status => {
            let (worktree_root, spine_dir) = find_repo_root()?;
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
                if !ms.conflicts.is_empty() {
                    println!("Unmerged paths:");
                    for c in &ms.conflicts {
                        println!("\tboth modified:   {}", c.path);
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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

                // Check for unresolved conflicts
                for c in &ms.conflicts {
                    let p = worktree_root.join(&c.path);
                    if p.exists() {
                        let content = fs::read_to_string(&p).unwrap_or_default();
                        if content.contains("<<<<<<< OURS") || content.contains(">>>>>>> THEIRS") {
                            bail!(
                                "fatal: Exiting because of unresolved conflict in '{}'",
                                c.path
                            );
                        }
                    }
                }

                // Stage any remaining resolved files
                for c in &ms.conflicts {
                    let _ = stage_path(
                        &worktree_root,
                        &worktree_root.join(&c.path),
                        &mut index,
                        &store,
                        &ignore,
                    );
                }
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
            let config = SpineConfig::load_from_file(spine_dir.join("spine.toml"))?;
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
    }

    Ok(())
}
