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

    /// PostgreSQL Database URL (e.g. postgres://spine:spine@127.0.0.1:5432/spine_forge)
    #[arg(long)]
    database_url: Option<String>,

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
        /// PostgreSQL Database URL
        #[arg(long)]
        database_url: Option<String>,
    },

    /// One-time bootstrap of the initial system administrator
    BootstrapAdmin {
        /// Administrator username
        #[arg(short, long)]
        username: String,

        /// Administrator email
        #[arg(short, long)]
        email: String,

        /// PostgreSQL Database URL
        #[arg(long)]
        database_url: Option<String>,

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

        /// Optional PostgreSQL Database URL
        #[arg(long)]
        database_url: Option<String>,

        /// Optional AEAD encryption passphrase (enables AES-256-GCM + PBKDF2)
        #[arg(long)]
        passphrase: Option<String>,

        /// Key rotation version (default: 1)
        #[arg(long, default_value_t = 1)]
        key_version: u16,
    },

    /// Restore and verify a backup archive into target directory
    Restore {
        /// Input archive file path (.tar.gz or .spinebak)
        #[arg(short, long)]
        input: PathBuf,

        /// Target storage root directory to restore into
        #[arg(long, default_value = "./.spine-dev-cluster/node-1")]
        target_storage: PathBuf,

        /// Target PostgreSQL Database URL for database restoration
        #[arg(long)]
        target_database_url: Option<String>,

        /// AEAD encryption passphrase if restoring an encrypted .spinebak archive
        #[arg(long)]
        passphrase: Option<String>,
    },

    /// Rotate the encryption key/passphrase of an encrypted backup archive without disk writes
    Rotate {
        /// Input encrypted archive path
        #[arg(short, long)]
        input: PathBuf,

        /// Output rotated archive path
        #[arg(short, long)]
        output: PathBuf,

        /// Current/old encryption passphrase
        #[arg(long)]
        old_passphrase: String,

        /// New encryption passphrase
        #[arg(long)]
        new_passphrase: String,

        /// New key rotation version (default: 2)
        #[arg(long, default_value_t = 2)]
        key_version: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Migrate { database_url }) => {
            let db_url = database_url
                .or_else(|| std::env::var("DATABASE_URL").ok())
                .ok_or_else(|| anyhow::anyhow!("DATABASE_URL must be provided for migration"))?;
            println!("Connecting to PostgreSQL database: {}", db_url);
            let _pg = PgDatabase::connect_and_migrate(&db_url).await?;
            println!("PostgreSQL schema migrations executed and verified successfully.");
            Ok(())
        }
        Some(Commands::BootstrapAdmin {
            username,
            email,
            database_url,
            profile,
            in_memory_db,
        }) => {
            let raw_prof = profile
                .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
                .unwrap_or_else(|| "core-forge-v1".to_string());
            let prof = spine_api::DeploymentProfile::from_str_relaxed(&raw_prof)
                .map_err(|e| anyhow::anyhow!(e))?;

            let db_url = database_url.or_else(|| std::env::var("DATABASE_URL").ok());
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
                database_url,
                passphrase,
                key_version,
            } => {
                let db_url = database_url.or_else(|| std::env::var("DATABASE_URL").ok());
                let db_snapshot_json = if let Some(url) = db_url {
                    println!("Connecting to PostgreSQL to capture snapshot: {}...", url);
                    let pool = sqlx::PgPool::connect(&url).await.map_err(|e| {
                        anyhow::anyhow!("Failed to connect to PostgreSQL for backup: {}", e)
                    })?;
                    let dump = spine_api::dump_database(&pool).await.map_err(|e| {
                        anyhow::anyhow!("Failed to dump PostgreSQL database: {}", e)
                    })?;
                    println!("PostgreSQL database snapshot captured successfully.");
                    Some(dump)
                } else {
                    let prof = std::env::var("SPINE_DEPLOYMENT_PROFILE").unwrap_or_default();
                    if prof == "core-forge-v1" {
                        anyhow::bail!(
                            "Under deployment profile 'core-forge-v1', PostgreSQL database URL is required for complete backups (--database-url or DATABASE_URL)."
                        );
                    }
                    None
                };

                println!(
                    "Creating coordinated backup from {} to {}",
                    storage_root.display(),
                    output.display()
                );
                let manifest = if let Some(pass) = passphrase {
                    println!(
                        "Applying SPNEBAK v2 AES-256-GCM AEAD streaming encryption (key version {})...",
                        key_version
                    );
                    spine_api::backup::BackupManager::create_encrypted_backup(
                        &storage_root,
                        db_snapshot_json.as_deref(),
                        &output,
                        &pass,
                        key_version,
                    )?
                } else {
                    spine_api::backup::BackupManager::create_backup(
                        &storage_root,
                        db_snapshot_json.as_deref(),
                        &output,
                    )?
                };
                println!("Backup archive created successfully:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Total files:   {}", manifest.total_files);
                println!("  Total bytes:   {}", manifest.total_bytes);
                println!("  Status:        {}", manifest.integrity_status);
                Ok(())
            }
            BackupCommands::Restore {
                input,
                target_storage,
                target_database_url,
                passphrase,
            } => {
                println!(
                    "Restoring backup archive from {} into {}",
                    input.display(),
                    target_storage.display()
                );
                let (manifest, report, db_snapshot) = if let Some(pass) = passphrase {
                    println!(
                        "Decrypting with AEAD passphrase and verifying chunk integrity tags..."
                    );
                    spine_api::backup::BackupManager::restore_encrypted_backup(
                        &input,
                        &target_storage,
                        &pass,
                    )?
                } else {
                    spine_api::backup::BackupManager::restore_backup(&input, &target_storage)?
                };
                println!("Storage restoration completed:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Files restored:{}", report.files_restored);
                println!("  Bytes restored:{}", report.bytes_restored);

                let target_db = target_database_url
                    .or_else(|| std::env::var("TARGET_DATABASE_URL").ok())
                    .or_else(|| std::env::var("DATABASE_URL").ok());

                if let Some(snapshot_json) = db_snapshot {
                    if let Some(db_url) = target_db {
                        println!(
                            "Restoring database snapshot into target database: {}...",
                            db_url
                        );
                        let pool = sqlx::PgPool::connect(&db_url).await.map_err(|e| {
                            anyhow::anyhow!(
                                "Failed to connect to target PostgreSQL for restore: {}",
                                e
                            )
                        })?;

                        let empty = spine_api::is_database_empty(&pool).await.map_err(|e| {
                            anyhow::anyhow!("Failed to inspect target database state: {}", e)
                        })?;
                        if !empty {
                            anyhow::bail!(
                                "Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption."
                            );
                        }

                        let db_report = spine_api::restore_database(&pool, &snapshot_json)
                            .await
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to restore database snapshot: {}", e)
                            })?;
                        println!("Database restore completed successfully:");
                        println!("  Tables restored: {}", db_report.tables_restored);
                        println!("  Rows restored:   {}", db_report.rows_restored);
                        println!("  Duration:        {} ms", db_report.duration_ms);
                    } else {
                        println!("WARNING: Backup archive contains a database snapshot, but no --target-database-url (or TARGET_DATABASE_URL/DATABASE_URL) was specified. Database was not restored.");
                    }
                }
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
                old_passphrase,
                new_passphrase,
                key_version,
            } => {
                println!(
                    "Rotating backup encryption key from {} to {} (target key version {})...",
                    input.display(),
                    output.display(),
                    key_version
                );
                spine_api::backup::BackupManager::rotate_backup_key(
                    &input,
                    &output,
                    &old_passphrase,
                    &new_passphrase,
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

    let database_url = args
        .database_url
        .or_else(|| std::env::var("DATABASE_URL").ok());
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
                "CRITICAL STARTUP ERROR: Mandatory PostgreSQL database connection string missing. Under deployment profile '{}', 'DATABASE_URL' (or --database-url) must be provided.",
                profile
            );
        }
    }

    // 1. Initialize Database backend
    let db: Arc<dyn Database> = match database_url {
        Some(db_url) => {
            tracing::info!(
                "Connecting to PostgreSQL database and applying migrations under profile '{}'...",
                profile
            );
            let pg_db = PgDatabase::connect_and_migrate(&db_url)
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Failed to connect to or migrate PostgreSQL database: {}", e)
                })?;

            // Immediate connectivity check
            pg_db.ping().await.map_err(|e| {
                anyhow::anyhow!(
                    "PostgreSQL database liveness check failed immediately after connect: {}",
                    e
                )
            })?;

            Arc::new(pg_db)
        }
        None => {
            if profile.permits_in_memory_db() && args.in_memory_db {
                tracing::warn!(
                    "Initializing Spine Forge with InMemoryDatabase adapter (PERMITTED ONLY for profile '{}')",
                    profile
                );
                Arc::new(InMemoryDatabase::new())
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
    let webhooks = Arc::new(WebhookService::new(Arc::clone(&db), ssrf.clone()));
    let webhook_worker = Arc::new(WebhookOutboxWorker::new(Arc::clone(&db), ssrf));
    let scheduler = Arc::new(
        WorkflowScheduler::new(Arc::clone(&db), Some((*webhooks).clone())).with_profile(profile),
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
        .unwrap_or_else(|_| format!("http://{}:{}", args.bind, args.port));

    let consistency_barrier = Arc::new(spine_api::backup::ConsistencyBarrier::new());

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
        consistency_barrier,
    };

    // 4. Spawn background provisioning worker
    let worker = Arc::new(ProvisioningWorker::new(
        Arc::clone(&db),
        coordinator,
        &storage_root_canonical,
    ));
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
