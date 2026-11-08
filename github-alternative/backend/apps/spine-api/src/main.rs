use clap::{Parser, Subcommand};
use rand::Rng;
use spine_api::{
    auth::{hash_password, DevMailCatcher, RateLimiter},
    build_router,
    core::types::{AuditEvent, User},
    AppState, ArtifactService, CargoRegistryAdapter, CoordinatorClient, Database, InMemoryDatabase,
    IsolatedRunnerExecutor, NativeVcsService, PackageService, PermissionService, PgDatabase,
    ProvisioningWorker, ReleaseService, RepositoryService, RunnerService, SsrfValidator,
    WebhookOutboxWorker, WebhookService, WorkflowScheduler, WorkflowService,
};
use std::io::Read;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(
    name = "spine-api",
    author = "Spine Project Engineers",
    version = "0.1.0",
    about = "Spine Forge User-Facing Application & Control Plane Server"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Bind IP address
    #[arg(short, long, default_value = "127.0.0.1")]
    bind: String,

    /// Listening port
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// SSH Listening port (e.g. 2222)
    #[arg(long, default_value_t = 2222)]
    ssh_port: u16,

    /// Path to file containing PostgreSQL Database URL
    #[arg(long)]
    database_url_file: Option<PathBuf>,

    /// Path to file containing PostgreSQL Migration Database URL (DDL role)
    #[arg(long)]
    migration_database_url_file: Option<PathBuf>,

    /// Deployment profile (core-forge-v1, development, test)
    #[arg(long)]
    profile: Option<String>,

    /// Force in-memory database for local zero-config development and testing
    #[arg(long, default_value_t = false)]
    in_memory_db: bool,

    /// Spine Coordinator RPC address (e.g. 127.0.0.1:9000)
    #[arg(long)]
    coordinator_addr: Option<String>,

    /// Storage root hosting native repositories
    #[arg(long, default_value = "./.spine-dev-cluster/node-1")]
    storage_root: PathBuf,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Start the Spine Forge API and SSH server (default)
    Serve,

    /// Run pending database migrations against PostgreSQL
    Migrate {
        /// Path to file containing PostgreSQL Database URL
        #[arg(long)]
        database_url_file: Option<PathBuf>,

        /// Path to file containing PostgreSQL Migration Database URL (DDL role)
        #[arg(long)]
        migration_database_url_file: Option<PathBuf>,
    },

    /// One-time bootstrap of the initial system administrator
    BootstrapAdmin {
        /// Administrator username
        #[arg(short, long)]
        username: String,

        /// Administrator email
        #[arg(short, long)]
        email: String,

        /// Path to file containing PostgreSQL Database URL
        #[arg(long)]
        database_url_file: Option<PathBuf>,

        /// Deployment profile
        #[arg(long)]
        profile: Option<String>,

        /// Use in-memory database (for testing)
        #[arg(long, default_value_t = false)]
        in_memory_db: bool,
    },

    /// Coordinated backup and disaster recovery commands
    Backup {
        #[command(subcommand)]
        action: BackupCommands,
    },
}

#[derive(Subcommand, Debug)]
enum BackupCommands {
    /// Create a full SHA-256 verified backup archive (.tar.gz or AEAD encrypted .spinebak)
    Create {
        /// Destination archive file path (.tar.gz or .spinebak)
        #[arg(short, long)]
        output: PathBuf,

        /// Storage root containing native repositories and metadata
        #[arg(long, default_value = "./.spine-dev-cluster/node-1")]
        storage_root: PathBuf,

        /// Path to file containing PostgreSQL Database URL
        #[arg(long)]
        database_url_file: Option<PathBuf>,

        /// Path to file containing AEAD encryption passphrase
        #[arg(long)]
        passphrase_file: Option<PathBuf>,

        /// Key rotation version (default: 1)
        #[arg(long, default_value_t = 1)]
        key_version: u16,

        /// Deployment profile (core-forge-v1, development, test)
        #[arg(long)]
        profile: Option<String>,
    },

    /// Restore and verify a backup archive into target directory
    Restore {
        /// Input archive file path (.tar.gz or .spinebak)
        #[arg(short, long)]
        input: PathBuf,

        /// Target storage root directory to restore into
        #[arg(long, default_value = "./.spine-dev-cluster/node-1")]
        target_storage: PathBuf,

        /// Path to file containing target PostgreSQL Database URL
        #[arg(long)]
        target_database_url_file: Option<PathBuf>,

        /// Path to file containing AEAD encryption passphrase
        #[arg(long)]
        passphrase_file: Option<PathBuf>,

        /// Deployment profile (core-forge-v1, development, test)
        #[arg(long)]
        profile: Option<String>,
    },

    /// Rotate the encryption key/passphrase of an encrypted backup archive without disk writes
    Rotate {
        /// Input encrypted archive path
        #[arg(short, long)]
        input: PathBuf,

        /// Output rotated archive path
        #[arg(short, long)]
        output: PathBuf,

        /// Path to file containing current/old encryption passphrase
        #[arg(long)]
        old_passphrase_file: Option<PathBuf>,

        /// Path to file containing new encryption passphrase
        #[arg(long)]
        new_passphrase_file: Option<PathBuf>,

        /// New key rotation version (default: 2)
        #[arg(long, default_value_t = 2)]
        key_version: u16,
    },
}

pub fn sanitize_db_url(raw: &str) -> String {
    if let Ok(mut parsed) = url::Url::parse(raw) {
        if parsed.password().is_some() {
            let _ = parsed.set_password(Some("***"));
        }
        parsed.to_string()
    } else {
        raw.split('@')
            .next_back()
            .map(|host| format!("postgres://***@{}", host))
            .unwrap_or_else(|| "***".to_string())
    }
}

pub fn validate_public_origin(
    origin: &str,
    profile: spine_api::DeploymentProfile,
) -> Result<(), anyhow::Error> {
    let parsed = url::Url::parse(origin)
        .map_err(|e| anyhow::anyhow!("Invalid FORGE_PUBLIC_ORIGIN URL '{}': {}", origin, e))?;

    if profile == spine_api::DeploymentProfile::CoreForge {
        if parsed.scheme() != "https" {
            anyhow::bail!(
                "CRITICAL SECURITY CONFIGURATION ERROR: Under deployment profile '{}', FORGE_PUBLIC_ORIGIN must use HTTPS (got '{}'). Insecure HTTP ingress is strictly forbidden.",
                profile,
                origin
            );
        }
        if parsed.host_str().is_none() || parsed.host_str().unwrap().is_empty() {
            anyhow::bail!(
                "CRITICAL SECURITY CONFIGURATION ERROR: FORGE_PUBLIC_ORIGIN '{}' has an empty hostname.",
                origin
            );
        }
        if !parsed.username().is_empty() || parsed.password().is_some() {
            anyhow::bail!(
                "CRITICAL SECURITY CONFIGURATION ERROR: FORGE_PUBLIC_ORIGIN must not contain embedded userinfo or credentials."
            );
        }
    }
    Ok(())
}

fn read_secret(
    direct_arg: Option<String>,
    file_path: Option<&std::path::Path>,
    env_vars: &[&str],
) -> Result<Option<String>, anyhow::Error> {
    if let Some(val) = direct_arg {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }
    if let Some(path) = file_path {
        let content = std::fs::read_to_string(path).map_err(|e| {
            anyhow::anyhow!("Failed to read secret file '{}': {}", path.display(), e)
        })?;
        let trimmed = content.trim();
        if trimmed.is_empty() {
            anyhow::bail!("Secret file '{}' is empty", path.display());
        }
        return Ok(Some(trimmed.to_string()));
    }
    for var in env_vars {
        if let Ok(val) = std::env::var(var) {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                return Ok(Some(trimmed.to_string()));
            }
        }
    }
    Ok(None)
}

fn parse_cli_profile(
    profile_arg: Option<&str>,
) -> Result<spine_api::DeploymentProfile, anyhow::Error> {
    let raw = profile_arg
        .map(|s| s.to_string())
        .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
        .unwrap_or_else(|| "core-forge-v1".to_string());
    spine_api::DeploymentProfile::from_str_relaxed(&raw)
        .map_err(|e| anyhow::anyhow!("Invalid deployment profile '{}': {}", raw, e))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Migrate {
            database_url_file,
            migration_database_url_file,
        }) => {
            let db_url = read_secret(
                None,
                migration_database_url_file
                    .as_deref()
                    .or(database_url_file.as_deref()),
                &[
                    "MIGRATION_DATABASE_URL",
                    "SPINE_MIGRATION_DATABASE_URL",
                    "DATABASE_URL",
                    "SPINE_DATABASE_URL",
                ],
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "DATABASE_URL or MIGRATION_DATABASE_URL must be provided for migration"
                )
            })?;
            println!(
                "Connecting to PostgreSQL database for migration: {}",
                sanitize_db_url(&db_url)
            );
            let pool = sqlx::PgPool::connect(&db_url).await.map_err(|e| {
                anyhow::anyhow!("Failed to connect to PostgreSQL for migration: {}", e)
            })?;
            spine_api::db::migrations::run_migrations(&pool)
                .await
                .map_err(|e| anyhow::anyhow!("Failed to execute schema migrations: {}", e))?;
            pool.close().await;
            println!("PostgreSQL schema migrations executed and verified successfully.");
            Ok(())
        }
        Some(Commands::BootstrapAdmin {
            username,
            email,
            database_url_file,
            profile,
            in_memory_db,
        }) => {
            let raw_prof = profile
                .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
                .unwrap_or_else(|| "core-forge-v1".to_string());
            let prof = spine_api::DeploymentProfile::from_str_relaxed(&raw_prof)
                .map_err(|e| anyhow::anyhow!(e))?;

            let db_url = read_secret(
                None,
                database_url_file.as_deref(),
                &["DATABASE_URL", "SPINE_DATABASE_URL"],
            )?;
            let db: Arc<dyn Database> = if prof.requires_persistent_database() {
                if in_memory_db {
                    anyhow::bail!(
                        "Admin bootstrap cannot use '--in-memory-db' under deployment profile '{}'.",
                        prof
                    );
                }
                let url = db_url.ok_or_else(|| {
                    anyhow::anyhow!(
                        "Mandatory database URL missing for admin bootstrap under profile '{}'.",
                        prof
                    )
                })?;
                println!("Connecting to PostgreSQL database for admin bootstrap...");
                Arc::new(PgDatabase::connect_and_migrate(&url).await?)
            } else if in_memory_db || db_url.is_none() {
                println!("Initializing bootstrap with in-memory database...");
                Arc::new(InMemoryDatabase::new())
            } else {
                let url = db_url.unwrap();
                println!("Connecting to PostgreSQL database for admin bootstrap...");
                Arc::new(PgDatabase::connect_and_migrate(&url).await?)
            };

            // Single execution lockout check
            if db.get_user_by_username(&username).await?.is_some() {
                anyhow::bail!(
                    "User '{}' already exists. Admin bootstrap can only be executed once.",
                    username
                );
            }
            if db.get_user_by_email(&email).await?.is_some() {
                anyhow::bail!(
                    "Email '{}' is already in use. Admin bootstrap can only be executed once.",
                    email
                );
            }

            // Generate cryptographically secure random password (32 alphanumeric chars)
            let raw_password: String = rand::thread_rng()
                .sample_iter(&rand::distributions::Alphanumeric)
                .take(32)
                .map(char::from)
                .collect();

            let password_hash = hash_password(&raw_password)
                .map_err(|e| anyhow::anyhow!("Password hashing failed: {}", e))?;

            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let user_id = uuid::Uuid::new_v4().to_string();

            let admin_user = User {
                id: user_id.clone(),
                username: username.clone(),
                normalized_username: username.to_lowercase(),
                email: email.clone(),
                normalized_email: email.to_lowercase(),
                display_name: format!("{} (Administrator)", username),
                password_hash,
                account_status: "active".to_string(),
                avatar_url: None,
                created_at_nanos: now,
                updated_at_nanos: now,
                last_login_nanos: None,
            };

            db.create_user(admin_user).await?;

            let audit = AuditEvent {
                id: 0,
                user_id: Some(user_id.clone()),
                action: "system.bootstrap_admin".to_string(),
                resource_type: "user".to_string(),
                resource_id: user_id.clone(),
                ip_address: Some("127.0.0.1".to_string()),
                metadata: serde_json::json!({
                    "username": username,
                    "email": email,
                    "source": "cli_bootstrap_admin"
                }),
                timestamp_nanos: now,
            };
            let _ = db.record_audit_event(audit).await;

            println!(
                "================================================================================"
            );
            println!(
                "                    SPINE FORGE INITIAL ADMIN BOOTSTRAP                         "
            );
            println!(
                "================================================================================"
            );
            println!("Administrator account bootstrapped successfully.");
            println!();
            println!("  User ID:   {}", user_id);
            println!("  Username:  {}", username);
            println!("  Email:     {}", email);
            println!("  Password:  {}", raw_password);
            println!();
            println!("CRITICAL: Save this temporary generated password securely now.");
            println!("It will NOT be displayed again and bootstrap cannot be re-executed.");
            println!(
                "================================================================================"
            );
            Ok(())
        }
        Some(Commands::Backup { action }) => match action {
            BackupCommands::Create {
                output,
                storage_root,
                database_url_file,
                passphrase_file,
                key_version,
                profile,
            } => {
                let prof = parse_cli_profile(profile.as_deref())?;
                let db_url = read_secret(
                    None,
                    database_url_file.as_deref(),
                    &["DATABASE_URL", "SPINE_DATABASE_URL"],
                )?;
                let pass = read_secret(
                    None,
                    passphrase_file.as_deref(),
                    &["SPINE_BACKUP_PASSPHRASE", "BACKUP_PASSPHRASE"],
                )?;

                let tmp_snap_dir = tempfile::Builder::new().prefix("spine_snap_").tempdir()?;
                let snap_file_path = tmp_snap_dir.path().join("snapshot.jsonl");

                let (_guard_barrier, _maintenance_guard) = if let Some(url) = db_url {
                    println!(
                        "Connecting to PostgreSQL to coordinate maintenance barrier: {}...",
                        sanitize_db_url(&url)
                    );
                    let pool = sqlx::PgPool::connect(&url).await.map_err(|e| {
                        anyhow::anyhow!("Failed to connect to PostgreSQL for backup: {}", e)
                    })?;

                    // Coordinate with cluster via database-backed maintenance barrier
                    let barrier = spine_api::backup::ConsistencyBarrier::with_pool(pool.clone());
                    let guard = barrier
                        .enter_maintenance(std::time::Duration::from_secs(30))
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to enter cluster maintenance: {}", e)
                        })?;

                    let mut snap_file = std::fs::File::create(&snap_file_path)?;
                    spine_api::dump_database_to_writer(&pool, &mut snap_file)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("Failed to stream PostgreSQL database dump: {}", e)
                        })?;
                    println!("PostgreSQL streaming snapshot captured successfully.");
                    (Some(barrier), Some(guard))
                } else {
                    if prof.requires_persistent_database() {
                        anyhow::bail!(
                            "Under deployment profile '{}', a PostgreSQL database URL is required for complete backups (--database-url, --database-url-file, or DATABASE_URL).",
                            prof
                        );
                    }
                    (None, None)
                };

                let has_db_snap = _maintenance_guard.is_some();

                println!(
                    "Creating coordinated backup from {} to {}",
                    storage_root.display(),
                    output.display()
                );
                let snap_path_opt = if has_db_snap {
                    Some(snap_file_path.as_path())
                } else {
                    None
                };
                let manifest = if let Some(pass_str) = pass {
                    println!(
                        "Applying SPNEBAK v2 AES-256-GCM AEAD streaming encryption (key version {})...",
                        key_version
                    );
                    spine_api::backup::BackupManager::create_encrypted_backup_from_file(
                        &storage_root,
                        snap_path_opt,
                        &output,
                        &pass_str,
                        key_version,
                    )?
                } else {
                    spine_api::backup::BackupManager::create_backup_from_file(
                        &storage_root,
                        snap_path_opt,
                        &output,
                    )?
                };
                println!("Backup archive created successfully:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Total files:   {}", manifest.total_files);
                println!("  Total bytes:   {}", manifest.total_bytes);
                println!("  Status:        {}", manifest.integrity_status);

                if let Some(guard) = _maintenance_guard {
                    let _ = guard.release().await;
                }
                Ok(())
            }
            BackupCommands::Restore {
                input,
                target_storage,
                target_database_url_file,
                passphrase_file,
                profile,
            } => {
                let prof = parse_cli_profile(profile.as_deref())?;

                if !input.exists() {
                    anyhow::bail!("Input backup archive '{}' does not exist", input.display());
                }

                let pass = read_secret(
                    None,
                    passphrase_file.as_deref(),
                    &["SPINE_BACKUP_PASSPHRASE", "BACKUP_PASSPHRASE"],
                )?;
                let target_db = read_secret(
                    None,
                    target_database_url_file.as_deref(),
                    &["TARGET_DATABASE_URL", "DATABASE_URL"],
                )?;

                // Check magic bytes of input archive before touching target storage
                let mut magic_buf = [0u8; 8];
                let mut f_chk = std::fs::File::open(&input).map_err(|e| {
                    anyhow::anyhow!("Failed to open archive '{}': {}", input.display(), e)
                })?;
                f_chk
                    .read_exact(&mut magic_buf)
                    .map_err(|e| anyhow::anyhow!("Failed to read archive magic bytes: {}", e))?;
                drop(f_chk);

                let is_encrypted = &magic_buf == b"SPNEBAK\x01" || &magic_buf == b"SPNEBAK\x02";
                if is_encrypted && pass.is_none() {
                    anyhow::bail!(
                        "Input archive is encrypted (SPNEBAK format), but no passphrase was provided (--passphrase-file, or SPINE_BACKUP_PASSPHRASE)."
                    );
                }

                if prof.requires_persistent_database() && target_db.is_none() {
                    anyhow::bail!(
                        "Under deployment profile '{}', a target PostgreSQL database URL is required for restore (--target-database-url-file, or TARGET_DATABASE_URL).",
                        prof
                    );
                }

                // FAIL-SAFE: Validate target database BEFORE modifying repository storage on disk
                let mut target_db_pool = None;
                if let Some(ref db_url) = target_db {
                    println!(
                        "Connecting and verifying target database: {}...",
                        sanitize_db_url(db_url)
                    );
                    let pool = sqlx::PgPool::connect(db_url).await.map_err(|e| {
                        anyhow::anyhow!("Failed to connect to target PostgreSQL for restore: {}", e)
                    })?;

                    let empty = spine_api::is_database_empty(&pool).await.map_err(|e| {
                        anyhow::anyhow!("Failed to inspect target database state: {}", e)
                    })?;
                    if !empty {
                        anyhow::bail!(
                            "Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption."
                        );
                    }
                    target_db_pool = Some(pool);
                }

                println!(
                    "Staging backup archive from {} into isolated staging directory for {}...",
                    input.display(),
                    target_storage.display()
                );
                let mut session = spine_api::backup::BackupManager::prepare_restore_session(
                    &input,
                    &target_storage,
                    pass.as_deref(),
                )?;

                println!("Archive staged and all checksums verified against manifest.");

                // If backup contains a database snapshot, restore database BEFORE publishing storage
                if let Some(ref snap_path) = session.db_snapshot_path {
                    if let Some(ref pool) = target_db_pool {
                        println!("Restoring streaming database snapshot into target database...");
                        let file = std::fs::File::open(snap_path).map_err(|e| {
                            anyhow::anyhow!("Failed to open extracted database snapshot: {}", e)
                        })?;
                        let mut reader = std::io::BufReader::new(file);
                        let db_report = match spine_api::restore_database_from_reader(
                            pool,
                            &mut reader,
                        )
                        .await
                        {
                            Ok(r) => {
                                spine_api::backup::RestoreJournal::record_db_generation(
                                    pool,
                                    &session.journal.target_generation,
                                    &session.manifest.backup_id,
                                )
                                .await
                                .map_err(|e| {
                                    anyhow::anyhow!("Failed to record database generation: {}", e)
                                })?;
                                session
                                    .journal
                                    .persist(spine_api::backup::RestorePhase::DbPublished)
                                    .map_err(|e| {
                                        anyhow::anyhow!("Failed to update restore journal: {}", e)
                                    })?;
                                r
                            }
                            Err(e) => {
                                let _ = session.abort(&format!("Database restore failed: {}", e));
                                anyhow::bail!("Failed to restore database snapshot: {}. Target repository storage was left untouched.", e);
                            }
                        };
                        println!("Database restore completed successfully:");
                        println!("  Tables restored: {}", db_report.tables_restored);
                        println!("  Rows restored:   {}", db_report.rows_restored);
                        println!("  Duration:        {} ms", db_report.duration_ms);
                    } else if prof.requires_persistent_database() {
                        let _ = session.abort("Target database required under current profile");
                        anyhow::bail!(
                            "Under deployment profile '{}', a target PostgreSQL database URL is required for restoring backups containing database snapshots (--target-database-url-file, or TARGET_DATABASE_URL).",
                            prof
                        );
                    } else {
                        println!("WARNING: Backup archive contains a database snapshot, but no target database URL was specified. Database was not restored.");
                    }
                }

                println!("Atomically publishing verified repository storage to live root...");
                let manifest = session.manifest.clone();
                let report = session.publish_storage().map_err(|e| {
                    anyhow::anyhow!("Failed to publish verified storage to live root: {}", e)
                })?;

                println!("Restoration sequence completed successfully:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Files restored:{}", report.files_restored);
                println!("  Bytes restored:{}", report.bytes_restored);
                println!(
                    "  Integrity:     {}",
                    if report.integrity_verified {
                        "VERIFIED (SHA-256 matched)"
                    } else {
                        "FAILED"
                    }
                );
                Ok(())
            }
            BackupCommands::Rotate {
                input,
                output,
                old_passphrase_file,
                new_passphrase_file,
                key_version,
            } => {
                let old_pass = read_secret(
                    None,
                    old_passphrase_file.as_deref(),
                    &["SPINE_BACKUP_OLD_PASSPHRASE", "BACKUP_OLD_PASSPHRASE"],
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("Current/old passphrase required (--old-passphrase-file, or SPINE_BACKUP_OLD_PASSPHRASE)")
                })?;

                let new_pass = read_secret(
                    None,
                    new_passphrase_file.as_deref(),
                    &["SPINE_BACKUP_NEW_PASSPHRASE", "BACKUP_NEW_PASSPHRASE"],
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("New passphrase required (--new-passphrase-file, or SPINE_BACKUP_NEW_PASSPHRASE)")
                })?;

                println!(
                    "Rotating backup encryption key from {} to {} (target key version {})...",
                    input.display(),
                    output.display(),
                    key_version
                );
                spine_api::backup::BackupManager::rotate_backup_key(
                    &input,
                    &output,
                    &old_pass,
                    &new_pass,
                    key_version,
                )?;
                println!(
                    "Key rotation completed successfully. New archive written to {}",
                    output.display()
                );
                Ok(())
            }
        },
        Some(Commands::Serve) | None => run_server(cli).await,
    }
}

async fn run_server(args: Cli) -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "spine_api=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let database_url = read_secret(
        None,
        args.database_url_file.as_deref(),
        &["DATABASE_URL", "SPINE_DATABASE_URL"],
    )?;

    let migration_database_url = read_secret(
        None,
        args.migration_database_url_file.as_deref(),
        &["MIGRATION_DATABASE_URL", "SPINE_MIGRATION_DATABASE_URL"],
    )?;

    let coordinator_addr = args
        .coordinator_addr
        .or_else(|| std::env::var("COORDINATOR_ADDR").ok());

    let raw_profile = args
        .profile
        .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
        .unwrap_or_else(|| "core-forge-v1".to_string());
    let profile = spine_api::DeploymentProfile::from_str_relaxed(&raw_profile)
        .map_err(|e| anyhow::anyhow!(e))?;

    tracing::info!(
        "Active deployment profile: {} (Requires PostgreSQL: {}, Untrusted CI: {})",
        profile,
        profile.requires_persistent_database(),
        if profile.allow_untrusted_ci() {
            "ENABLED"
        } else {
            "DISABLED"
        }
    );

    // Profile enforcement: CoreForge mandates PostgreSQL and strictly forbids in-memory fallback
    if profile.requires_persistent_database() {
        if args.in_memory_db {
            anyhow::bail!(
                "CRITICAL STARTUP ERROR: '--in-memory-db' is strictly forbidden under deployment profile '{}'. Real PostgreSQL persistence is required.",
                profile
            );
        }
        if database_url.is_none() {
            anyhow::bail!(
                "CRITICAL STARTUP ERROR: Mandatory PostgreSQL database connection string missing. Under deployment profile '{}', 'DATABASE_URL' (or --database-url / --database-url-file) must be provided.",
                profile
            );
        }
    }

    // 1. Initialize Database backend
    let (db, pg_pool): (Arc<dyn Database>, Option<sqlx::PgPool>) = match database_url {
        Some(db_url) => {
            if let Some(mig_url) = migration_database_url {
                tracing::info!(
                    "Running baseline schema migrations using dedicated migration credentials ({}) under profile '{}'...",
                    sanitize_db_url(&mig_url),
                    profile
                );
                let migrator_pool = sqlx::PgPool::connect(&mig_url).await.map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to connect to PostgreSQL with migration credentials: {}",
                        e
                    )
                })?;
                spine_api::db::migrations::run_migrations(&migrator_pool)
                    .await
                    .map_err(|e| anyhow::anyhow!("Failed to execute schema migrations: {}", e))?;
                migrator_pool.close().await;
                tracing::info!(
                    "Migrations complete. Dedicated migration connection closed and discarded."
                );

                tracing::info!(
                    "Connecting to PostgreSQL as runtime application role ({}) under profile '{}'...",
                    sanitize_db_url(&db_url),
                    profile
                );
                let pg_db = PgDatabase::connect(&db_url).await.map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to connect to PostgreSQL application database: {}",
                        e
                    )
                })?;
                pg_db.ping().await.map_err(|e| {
                    anyhow::anyhow!("PostgreSQL application role liveness check failed: {}", e)
                })?;
                let pool = pg_db.pool().clone();
                (Arc::new(pg_db), Some(pool))
            } else {
                tracing::info!(
                    "Connecting to PostgreSQL database and applying migrations under profile '{}'...",
                    profile
                );
                let pg_db = PgDatabase::connect_and_migrate(&db_url)
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to connect to or migrate PostgreSQL database: {}",
                            e
                        )
                    })?;

                // Immediate connectivity check
                pg_db.ping().await.map_err(|e| {
                    anyhow::anyhow!(
                        "PostgreSQL database liveness check failed immediately after connect: {}",
                        e
                    )
                })?;

                let pool = pg_db.pool().clone();
                (Arc::new(pg_db), Some(pool))
            }
        }
        None => {
            if profile.permits_in_memory_db() && args.in_memory_db {
                tracing::warn!(
                    "Initializing Spine Forge with InMemoryDatabase adapter (PERMITTED ONLY for profile '{}')",
                    profile
                );
                (Arc::new(InMemoryDatabase::new()), None)
            } else {
                anyhow::bail!(
                    "CRITICAL CONFIGURATION ERROR: No database backend configured. Under profile '{}', provide DATABASE_URL or set --in-memory-db explicitly.",
                    profile
                );
            }
        }
    };

    // 2. Initialize Coordinator Client
    let coordinator = coordinator_addr.map(|addr| {
        tracing::info!("Configured Spine Coordinator at {}", addr);
        CoordinatorClient::new(addr)
    });

    // 3. Ensure Storage Root exists
    std::fs::create_dir_all(&args.storage_root)?;
    let storage_root_canonical = std::fs::canonicalize(&args.storage_root)?;
    tracing::info!("Storage root: {}", storage_root_canonical.display());

    // 3.1 Restore state machine crash recovery & generation validation
    if let Some(msg) = spine_api::backup::RestoreJournal::handle_startup_recovery(
        &storage_root_canonical,
        pg_pool.as_ref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("FATAL RESTORE RECOVERY FAILURE: {}", e))?
    {
        tracing::warn!("Restore state machine recovery: {}", msg);
    }

    let permissions = Arc::new(PermissionService::new(Arc::clone(&db)));
    let repositories = Arc::new(RepositoryService::new(
        Arc::clone(&db),
        Arc::clone(&permissions),
    ));
    let vcs = Arc::new(NativeVcsService::new(&storage_root_canonical));
    let rate_limiter = Arc::new(RateLimiter::new());
    let mail = Arc::new(DevMailCatcher::new());

    let artifact_store_path = storage_root_canonical.join("artifacts");
    let artifact_store = Arc::new(spine_artifact::LocalArtifactStore::new(
        artifact_store_path,
    )?);
    let artifacts = Arc::new(ArtifactService::new(Arc::clone(&db), artifact_store));
    let releases = Arc::new(ReleaseService::new(Arc::clone(&db), Arc::clone(&artifacts)));
    let packages = Arc::new(PackageService::new(Arc::clone(&db), Arc::clone(&artifacts)));
    let cargo = Arc::new(CargoRegistryAdapter::new(Arc::clone(&packages)));

    let ssrf = SsrfValidator::new(false);
    let consistency_barrier = Arc::new(match pg_pool {
        Some(pool) => spine_api::backup::ConsistencyBarrier::with_pool(pool),
        None => spine_api::backup::ConsistencyBarrier::new(),
    });

    let webhooks = Arc::new(WebhookService::new(Arc::clone(&db), ssrf.clone()));
    let webhook_worker = Arc::new(
        WebhookOutboxWorker::new(Arc::clone(&db), ssrf)
            .with_barrier(Arc::clone(&consistency_barrier)),
    );
    let scheduler = Arc::new(
        WorkflowScheduler::new(Arc::clone(&db), Some((*webhooks).clone()))
            .with_profile(profile)
            .with_barrier(Arc::clone(&consistency_barrier)),
    );
    let workflows = Arc::new(
        WorkflowService::new(Arc::clone(&db), Some((*webhooks).clone())).with_profile(profile),
    );
    let runners =
        Arc::new(RunnerService::new(Arc::clone(&db), (*scheduler).clone()).with_profile(profile));
    let runner_workspace_root = storage_root_canonical.join("runner_workspaces");
    let runner_executor =
        Arc::new(IsolatedRunnerExecutor::new(runner_workspace_root).with_profile(profile));

    let search = Arc::new(spine_api::SearchService::new(
        Arc::clone(&db),
        Arc::clone(&permissions),
        storage_root_canonical.clone(),
    ));
    let discussions = Arc::new(spine_api::DiscussionService::new(
        Arc::clone(&db),
        Arc::clone(&permissions),
    ));
    let discovery = Arc::new(spine_api::DiscoveryService::new(
        Arc::clone(&db),
        Arc::clone(&permissions),
        storage_root_canonical.clone(),
    ));
    let security = Arc::new(spine_api::SecurityService::new(
        Arc::clone(&db),
        Arc::clone(&permissions),
        storage_root_canonical.clone(),
    ));

    let public_origin = std::env::var("FORGE_PUBLIC_ORIGIN")
        .or_else(|_| std::env::var("APP_URL"))
        .unwrap_or_else(|_| {
            if profile == spine_api::DeploymentProfile::CoreForge {
                "https://localhost".to_string()
            } else {
                format!("http://{}:{}", args.bind, args.port)
            }
        });

    validate_public_origin(&public_origin, profile)?;

    let state = AppState {
        profile,
        db: Arc::clone(&db),
        permissions: Arc::clone(&permissions),
        repositories: Arc::clone(&repositories),
        vcs: Arc::clone(&vcs),
        rate_limiter,
        mail,
        coordinator: coordinator.clone(),
        artifacts,
        releases,
        packages,
        cargo,
        webhooks,
        webhook_worker,
        workflows,
        scheduler,
        runners,
        runner_executor,
        search,
        discussions,
        discovery,
        security,
        public_origin,
        consistency_barrier: Arc::clone(&consistency_barrier),
    };

    // 4. Spawn background provisioning worker
    let worker = Arc::new(
        ProvisioningWorker::new(Arc::clone(&db), coordinator, &storage_root_canonical)
            .with_barrier(Arc::clone(&consistency_barrier)),
    );
    let worker_clone = Arc::clone(&worker);
    let worker_handle = tokio::spawn(async move {
        tracing::info!("Starting background repository provisioning worker");
        worker_clone.run().await;
    });

    // 5. Start HTTP and SSH servers
    let ssh_addr_str =
        std::env::var("SSH_ADDR").unwrap_or_else(|_| format!("{}:{}", args.bind, args.ssh_port));
    let ssh_addr: SocketAddr = ssh_addr_str.parse()?;
    let (_bound_ssh, _ssh_handle) = spine_api::start_ssh_server(state.clone(), ssh_addr).await?;

    let app = build_router(state);
    let addr: SocketAddr = format!("{}:{}", args.bind, args.port).parse()?;
    tracing::info!("Spine Forge API listening on http://{}", addr);

    let listener = tokio::net::TcpListener::bind(addr).await?;

    tokio::select! {
        res = axum::serve(listener, app) => {
            if let Err(e) = res {
                tracing::error!("Server error: {}", e);
            }
        }
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("Shutdown signal received, terminating gracefully...");
        }
    }

    worker.shutdown();
    let _ = worker_handle.await;
    tracing::info!("Spine Forge shutdown complete.");
    Ok(())
}
