use anyhow::{bail, Context, Result};
use chrono::{TimeZone, Utc};
use clap::{Parser, Subcommand};
use spine_config::SpineConfig;
use spine_core::{Commit, HashAlgorithm, ObjectId, ObjectType};
use spine_format::serialize_commit;
use spine_history::{get_commit, walk_history};
use spine_index::Index;
use spine_integrity::run_fsck;
use spine_object_store::{FileSystemObjectStore, ObjectStore};
use spine_worktree::{compute_status, flatten_tree, stage_path, write_index_to_tree, IgnoreRules};
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
    version = "0.1.0"
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
    /// Verify repository integrity and detect corruption
    Fsck,
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
                    config.set_value(&key, &value)?;
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

            if report.is_clean() {
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

            let root_tree_id = write_index_to_tree(&index, &store)?;

            let mut parents = Vec::new();
            if let Some(parent_cid) = state.head_commit_id {
                let parent_commit = get_commit(&store, &parent_cid)?;
                if parent_commit.root_tree == root_tree_id {
                    println!("nothing to commit, working tree clean");
                    return Ok(());
                }
                parents.push(parent_cid);
            }

            let now = Utc::now();
            let timestamp_nanos = now.timestamp_nanos_opt().unwrap_or(0);

            let commit = Commit {
                root_tree: root_tree_id,
                parents,
                author_name,
                author_email,
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
                let is_root = if commit.parents.is_empty() {
                    "(root-commit) "
                } else {
                    ""
                };
                println!("[{} {}{}] {}", branch, is_root, short_id, commit.message);
            } else {
                fs::write(spine_dir.join("HEAD"), format!("{}\n", commit_id.to_hex()))?;
                let short_id = &commit_id.to_hex()[..7];
                println!("[detached HEAD {}] {}", short_id, commit.message);
            }
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
            let state = resolve_head(&spine_dir)?;

            let target_cid = if object == "HEAD" {
                match state.head_commit_id {
                    Some(cid) => cid,
                    None => bail!("No commits yet"),
                }
            } else {
                ObjectId::from_hex(&object).context("Invalid commit hash")?
            };

            let commit = get_commit(&store, &target_cid)?;
            println!("commit {}", target_cid);
            println!("Author: {} <{}>", commit.author_name, commit.author_email);
            let dt = Utc.timestamp_nanos(commit.timestamp_nanos);
            println!("Date:   {}", dt.to_rfc2822());
            println!("Tree:   {}", commit.root_tree);
            println!();
            for line in commit.message.lines() {
                println!("    {}", line);
            }
            println!();

            // Display contents of root tree
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
