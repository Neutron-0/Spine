use clap::{Parser, Subcommand};
use rand::Rng;
use spine_api::{
    auth::{hash_password, DevMailCatcher, RateLimiter},
    build_router,
    core::types::{AuditEvent, PersonalAccessToken, User},
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

        /// Path to file to write generated administrative token
        #[arg(long)]
        token_out_file: Option<PathBuf>,

        /// Path to file to write generated administrative initial password
        #[arg(long)]
        password_out_file: Option<PathBuf>,

        /// Use in-memory database (for testing)
        #[arg(long, default_value_t = false)]
        in_memory_db: bool,
    },

    /// Rotate administrative personal access token securely
    RotateAdminToken {
        /// Administrator username
        #[arg(short, long)]
        username: String,

        /// Path to file containing PostgreSQL Database URL
        #[arg(long)]
        database_url_file: Option<PathBuf>,

        /// Deployment profile
        #[arg(long)]
        profile: Option<String>,

        /// Path to file to write rotated administrative token
        #[arg(long)]
        token_out_file: PathBuf,
    },

    /// Compute cryptographic BLAKE3 hash of a token from stdin
    HashToken,

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

        /// Spine coordinator RPC address (e.g. 127.0.0.1:9000 or COORDINATOR_ADDR)
        #[arg(long)]
        coordinator_addr: Option<String>,

        /// Comma-separated storage node endpoints (e.g. "storage-1=127.0.0.1:9001,storage-2=127.0.0.1:9002")
        #[arg(long)]
        storage_node_addrs: Option<String>,
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

        /// Spine coordinator RPC address (e.g. 127.0.0.1:9000 or COORDINATOR_ADDR)
        #[arg(long)]
        coordinator_addr: Option<String>,

        /// Comma-separated storage node endpoints (e.g. "storage-1=127.0.0.1:9001,storage-2=127.0.0.1:9002")
        #[arg(long)]
        storage_node_addrs: Option<String>,

        /// Explicitly permit restoring unencrypted plaintext archives during legacy migration
        #[arg(long, default_value_t = false)]
        allow_unencrypted_legacy_restore: bool,
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

    /// Encrypt a file or stream into SPNEBAK v2 authenticated AEAD format (AES-256-GCM)
    Encrypt {
        /// Input plaintext file or archive path (use '-' for stdin)
        #[arg(short, long)]
        input: PathBuf,

        /// Output encrypted archive path (.spnebak, use '-' for stdout)
        #[arg(short, long)]
        output: PathBuf,

        /// Path to file containing AEAD encryption passphrase
        #[arg(long)]
        passphrase_file: Option<PathBuf>,

        /// Key rotation version (default: 1)
        #[arg(long, default_value_t = 1)]
        key_version: u16,
    },

    /// Decrypt and authenticate a SPNEBAK v2 AEAD encrypted archive into plaintext
    Decrypt {
        /// Input encrypted archive path (.spnebak, use '-' for stdin)
        #[arg(short, long)]
        input: PathBuf,

        /// Output decrypted plaintext path (use '-' for stdout)
        #[arg(short, long)]
        output: PathBuf,

        /// Path to file containing AEAD encryption passphrase
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
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

fn resolve_postgres_url(
    direct_arg: Option<String>,
    file_path: Option<&std::path::Path>,
    env_vars: &[&str],
    user_var: Option<&str>,
    pass_var: Option<&str>,
    pass_file_var: Option<&str>,
) -> Result<Option<String>, anyhow::Error> {
    // Canonical database name contract & conflict detection
    let p_db = std::env::var("POSTGRES_DB").ok();
    let s_db = std::env::var("SPINE_POSTGRES_DB").ok();
    if let (Some(ref a), Some(ref b)) = (&p_db, &s_db) {
        if a.trim() != b.trim() {
            anyhow::bail!(
                "Conflicting PostgreSQL database name contract: POSTGRES_DB='{}' but SPINE_POSTGRES_DB='{}'. Both must match if specified.",
                a, b
            );
        }
    }

    if let Some(url) = read_secret(direct_arg, file_path, env_vars)? {
        return Ok(Some(url));
    }

    let host = std::env::var("POSTGRES_HOST").ok();
    let port = std::env::var("POSTGRES_PORT")
        .ok()
        .unwrap_or_else(|| "5432".to_string());
    let db = p_db.or(s_db).unwrap_or_else(|| "spine_forge".to_string());

    if let Some(host) = host {
        let user = user_var
            .and_then(|v| std::env::var(v).ok())
            .unwrap_or_else(|| "spine_app".to_string());

        let pass = if let Some(pf_var) = pass_file_var {
            if let Ok(pf) = std::env::var(pf_var) {
                let p = std::path::Path::new(&pf);
                if !p.exists() {
                    anyhow::bail!(
                        "Configured database password file '{}' ({}) does not exist",
                        pf,
                        pf_var
                    );
                }
                let content = std::fs::read_to_string(p).map_err(|e| {
                    anyhow::anyhow!(
                        "Failed to read database password file '{}' ({}): {}",
                        pf,
                        pf_var,
                        e
                    )
                })?;
                let trimmed = content.trim();
                if trimmed.is_empty() {
                    anyhow::bail!(
                        "Configured database password file '{}' ({}) is empty",
                        pf,
                        pf_var
                    );
                }
                Some(trimmed.to_string())
            } else {
                pass_var.and_then(|v| std::env::var(v).ok())
            }
        } else {
            pass_var.and_then(|v| std::env::var(v).ok())
        };

        if let Some(pass) = pass {
            let mut parsed_url = url::Url::parse(&format!("postgres://{}:{}/{}", host, port, db))?;
            parsed_url
                .set_username(&user)
                .map_err(|_| anyhow::anyhow!("Invalid database username"))?;
            parsed_url
                .set_password(Some(&pass))
                .map_err(|_| anyhow::anyhow!("Invalid database password"))?;
            return Ok(Some(parsed_url.to_string()));
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
            let db_url = resolve_postgres_url(
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
                Some("POSTGRES_MIGRATOR_USER"),
                Some("POSTGRES_MIGRATOR_PASSWORD"),
                Some("POSTGRES_MIGRATOR_PASSWORD_FILE"),
            )?
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "DATABASE_URL or MIGRATION_DATABASE_URL must be provided for migration"
                )
            })?;

            let storage_root = std::env::var("SPINE_STORAGE_ROOT")
                .or_else(|_| std::env::var("STORAGE_ROOT"))
                .ok()
                .map(PathBuf::from);

            let active_db_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                &db_url,
                storage_root.as_deref(),
            )
            .await
            .map_err(|e| {
                anyhow::anyhow!("Database selector resolution failed for migration: {}", e)
            })?;

            println!(
                "Connecting to PostgreSQL database for migration: {}",
                sanitize_db_url(&active_db_url)
            );
            let pool = sqlx::PgPool::connect(&active_db_url).await.map_err(|e| {
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
            token_out_file,
            password_out_file,
            in_memory_db,
        }) => {
            let raw_prof = profile
                .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
                .unwrap_or_else(|| "core-forge-v1".to_string());
            let prof = spine_api::DeploymentProfile::from_str_relaxed(&raw_prof)
                .map_err(|e| anyhow::anyhow!(e))?;

            let db_url = resolve_postgres_url(
                None,
                database_url_file.as_deref(),
                &["DATABASE_URL", "SPINE_DATABASE_URL"],
                Some("POSTGRES_APP_USER"),
                Some("POSTGRES_APP_PASSWORD"),
                Some("POSTGRES_APP_PASSWORD_FILE"),
            )?;

            let storage_root = std::env::var("SPINE_STORAGE_ROOT")
                .or_else(|_| std::env::var("STORAGE_ROOT"))
                .ok()
                .map(PathBuf::from);

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
                let active_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                    &url,
                    storage_root.as_deref(),
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Database selector resolution failed for bootstrap: {}", e)
                })?;
                println!(
                    "Connecting to PostgreSQL database for admin bootstrap ({})...",
                    sanitize_db_url(&active_url)
                );
                Arc::new(PgDatabase::connect(&active_url).await?)
            } else if in_memory_db || db_url.is_none() {
                println!("Initializing bootstrap with in-memory database...");
                Arc::new(InMemoryDatabase::new())
            } else {
                let url = db_url.unwrap();
                let active_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                    &url,
                    storage_root.as_deref(),
                )
                .await
                .map_err(|e| {
                    anyhow::anyhow!("Database selector resolution failed for bootstrap: {}", e)
                })?;
                println!(
                    "Connecting to PostgreSQL database for admin bootstrap ({})...",
                    sanitize_db_url(&active_url)
                );
                Arc::new(PgDatabase::connect(&active_url).await?)
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

            let raw_admin_token = format!("sp_pat_{}", uuid::Uuid::new_v4().simple());
            let pat = PersonalAccessToken {
                id: format!("pat_{}", uuid::Uuid::new_v4().simple()),
                user_id: user_id.clone(),
                name: "bootstrap-admin-token".to_string(),
                token_hash: spine_api::auth::pat::hash_pat(&raw_admin_token),
                scopes: vec!["admin".to_string(), "repo".to_string(), "write".to_string()],
                expires_at_nanos: i64::MAX,
                created_at_nanos: now,
                last_used_at_nanos: None,
                revoked: false,
            };
            db.create_pat(pat).await?;

            let target_token_file = token_out_file.or_else(|| {
                std::env::var("SPINE_ADMIN_TOKEN_FILE").ok().map(PathBuf::from)
            });
            let mut resolved_password_file = password_out_file.or_else(|| {
                std::env::var("SPINE_ADMIN_PASSWORD_FILE").ok().map(PathBuf::from)
            });
            if resolved_password_file.is_none() {
                if let Some(ref tp) = target_token_file {
                    resolved_password_file = Some(tp.with_file_name("admin_password"));
                }
            }

            let is_file_delivery = target_token_file.is_some() || resolved_password_file.is_some();

            if let Some(ref path) = target_token_file {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(path, &raw_admin_token)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }

            if let Some(ref path) = resolved_password_file {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                std::fs::write(path, &raw_password)?;
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
                }
            }

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
            println!("  User ID:       {}", user_id);
            println!("  Username:      {}", username);
            println!("  Email:         {}", email);
            if is_file_delivery {
                println!("  Password:      [SECURELY WRITTEN TO FILE - SUPPRESSED FROM STDOUT]");
                println!("  Admin Token:   [SECURELY WRITTEN TO FILE - SUPPRESSED FROM STDOUT]");
            } else {
                println!("  Password:      {}", raw_password);
                println!("  Admin Token:   {}", raw_admin_token);
            }
            if let Some(ref p) = target_token_file {
                println!("  Token File:    {:?}", p);
            }
            if let Some(ref p) = resolved_password_file {
                println!("  Password File: {:?}", p);
            }
            println!();
            if is_file_delivery {
                println!("CRITICAL: Administrative credentials written to protected output files.");
                println!("Verify file permissions (0600) and intake securely.");
            } else {
                println!("CRITICAL: Save this temporary generated password & token securely now.");
                println!("They will NOT be displayed again and bootstrap cannot be re-executed.");
            }
            println!(
                "================================================================================"
            );
            Ok(())
        }
        Some(Commands::RotateAdminToken {
            username,
            database_url_file,
            profile,
            token_out_file,
        }) => {
            let raw_prof = profile
                .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
                .unwrap_or_else(|| "core-forge-v1".to_string());
            let prof = spine_api::DeploymentProfile::from_str_relaxed(&raw_prof)
                .map_err(|e| anyhow::anyhow!(e))?;

            let db_url = resolve_postgres_url(
                None,
                database_url_file.as_deref(),
                &["DATABASE_URL", "SPINE_DATABASE_URL"],
                Some("POSTGRES_APP_USER"),
                Some("POSTGRES_APP_PASSWORD"),
                Some("POSTGRES_APP_PASSWORD_FILE"),
            )?;

            let db: Arc<dyn Database> = if prof.requires_persistent_database() {
                let active_url = db_url.ok_or_else(|| {
                    anyhow::anyhow!("PostgreSQL DATABASE_URL required for profile '{}'", prof)
                })?;
                Arc::new(PgDatabase::connect(&active_url).await?)
            } else {
                anyhow::bail!("RotateAdminToken requires a persistent database");
            };

            let user = db
                .get_user_by_username(&username)
                .await?
                .ok_or_else(|| anyhow::anyhow!("User '{}' not found", username))?;

            // Revoke all existing tokens for this user
            let pats = db.list_user_pats(&user.id).await?;
            for pat in pats {
                let _ = db.revoke_pat(&pat.id, &user.id).await;
            }

            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            let raw_admin_token = format!("sp_pat_{}", uuid::Uuid::new_v4().simple());
            let pat = PersonalAccessToken {
                id: format!("pat_{}", uuid::Uuid::new_v4().simple()),
                user_id: user.id.clone(),
                name: "rotated-admin-token".to_string(),
                token_hash: spine_api::auth::pat::hash_pat(&raw_admin_token),
                scopes: vec!["admin".to_string(), "repo".to_string(), "write".to_string()],
                expires_at_nanos: i64::MAX,
                created_at_nanos: now,
                last_used_at_nanos: None,
                revoked: false,
            };
            db.create_pat(pat).await?;

            if let Some(parent) = token_out_file.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            std::fs::write(&token_out_file, &raw_admin_token)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&token_out_file, std::fs::Permissions::from_mode(0o600));
            }

            println!(
                "Admin token successfully rotated for user '{}'. Token securely written to '{}'. [SECRET SUPPRESSED FROM STDOUT]",
                username,
                token_out_file.display()
            );
            Ok(())
        }
        Some(Commands::HashToken) => {
            use std::io::Read;
            let mut buf = String::new();
            std::io::stdin().read_to_string(&mut buf)?;
            let trimmed = buf.trim();
            let hash = spine_api::auth::pat::hash_pat(trimmed);
            println!("{}", hash);
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
                coordinator_addr,
                storage_node_addrs,
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

                let storage_root = if storage_root
                    == std::path::Path::new("./.spine-dev-cluster/node-1")
                {
                    if let Ok(r) = std::env::var("SPINE_STORAGE_ROOT")
                        .or_else(|_| std::env::var("STORAGE_ROOT"))
                    {
                        std::path::PathBuf::from(r.trim())
                    } else if prof.requires_persistent_database() {
                        anyhow::bail!(
                            "Under deployment profile '{}', the default development storage root ('./.spine-dev-cluster/node-1') is strictly forbidden. Configure STORAGE_ROOT or --storage-root.",
                            prof
                        );
                    } else {
                        storage_root
                    }
                } else {
                    storage_root
                };

                let coord_addr_opt = coordinator_addr
                    .or_else(|| std::env::var("SPINE_COORDINATOR_ADDR").ok())
                    .or_else(|| std::env::var("COORDINATOR_ADDR").ok());
                let storage_nodes_str_opt = storage_node_addrs
                    .or_else(|| std::env::var("SPINE_STORAGE_NODES").ok())
                    .or_else(|| std::env::var("STORAGE_NODE_ADDRS").ok())
                    .or_else(|| std::env::var("STORAGE_NODES").ok());

                let mut node_endpoints = Vec::new();
                if let Some(nodes_str) = storage_nodes_str_opt {
                    for part in nodes_str.split(',') {
                        let trimmed = part.trim();
                        if !trimmed.is_empty() {
                            if let Some((id, addr)) = trimmed.split_once('=') {
                                node_endpoints
                                    .push((id.trim().to_string(), addr.trim().to_string()));
                            } else {
                                node_endpoints.push((
                                    format!("node-{}", node_endpoints.len() + 1),
                                    trimmed.to_string(),
                                ));
                            }
                        }
                    }
                }

                if prof.requires_persistent_database() {
                    if coord_addr_opt.is_none() {
                        anyhow::bail!(
                            "Under deployment profile '{}', a coordinator address is required for complete cluster backups (--coordinator-addr, or COORDINATOR_ADDR).",
                            prof
                        );
                    }
                    if node_endpoints.len() < 3 {
                        anyhow::bail!(
                            "Under deployment profile '{}', all 3 storage nodes (node-1, node-2, node-3) are required for complete cluster backups (--storage-node-addrs, or STORAGE_NODE_ADDRS). Found {} nodes.",
                            prof,
                            node_endpoints.len()
                        );
                    }
                    if db_url.is_none() {
                        anyhow::bail!(
                            "Under deployment profile '{}', a PostgreSQL database URL is required for complete backups (--database-url, --database-url-file, or DATABASE_URL).",
                            prof
                        );
                    }
                    if pass.is_none() {
                        anyhow::bail!(
                            "Under deployment profile '{}', authenticated AEAD encryption is MANDATORY for backups. Passphrase must be provided (--passphrase-file, or SPINE_BACKUP_PASSPHRASE). Plaintext production backups containing SSH identity keys are strictly forbidden.",
                            prof
                        );
                    }
                }

                let (pg_pool_opt, guard_opt) = if let Some(url) = db_url {
                    let resolved_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                        &url,
                        Some(&storage_root),
                    )
                    .await
                    .map_err(|e| {
                        anyhow::anyhow!("Database selector resolution failed for backup: {}", e)
                    })?;

                    println!(
                        "Connecting to PostgreSQL to coordinate maintenance barrier: {}...",
                        sanitize_db_url(&resolved_url)
                    );
                    let pool = sqlx::PgPool::connect(&resolved_url).await.map_err(|e| {
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

                    (Some(pool), Some(guard))
                } else {
                    (None, None)
                };

                let fencing_generation = guard_opt
                    .as_ref()
                    .map(|g| g.fencing_generation)
                    .unwrap_or(1);
                let owner_token = guard_opt.as_ref().map(|g| g.owner_token.clone());
                let cancel_token = guard_opt.as_ref().map(|g| g.cancel_token());
                let lease_expires_at_nanos = guard_opt.as_ref().map(|g| g.current_lease_expiry());
                let lease_expires_atomic = guard_opt.as_ref().map(|g| g.lease_expires_atomic());

                let guard_shared: Option<
                    std::sync::Arc<tokio::sync::Mutex<Option<spine_api::backup::MaintenanceGuard>>>,
                > = guard_opt.map(|g| std::sync::Arc::new(tokio::sync::Mutex::new(Some(g))));

                let stream_participants: Vec<(
                    spine_api::BackupParticipantRecord,
                    spine_api::backup::ParticipantPayloadSource,
                )> = Vec::new();
                let guard_shared_for_release = guard_shared.clone();
                let release_db_guard = guard_shared_for_release.map(|shared| {
                    move || async move {
                        let mut g_opt = shared.lock().await;
                        if let Some(guard) = g_opt.take() {
                            guard.release().await
                        } else {
                            Ok(())
                        }
                    }
                });
                let lease_storage_root = storage_root.clone();
                let cancel_token_work = cancel_token.clone();
                let guard_shared_for_publish = guard_shared.clone();
                spine_api::backup::run_backup_with_maintenance_cleanup_with_lease(
                    Some(&lease_storage_root),
                    fencing_generation,
                    coord_addr_opt.as_deref(),
                    &node_endpoints,
                    owner_token.as_deref(),
                    lease_expires_at_nanos,
                    lease_expires_atomic,
                    cancel_token.clone(),
                    release_db_guard,
                    |coord_client, storage_clients| async move {
                if let Some(ref ct) = cancel_token_work {
                    if ct.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(spine_api::core::error::ForgeError::ServiceUnavailable(
                            "Backup aborted: database maintenance barrier lock or connection lost".into(),
                        ));
                    }
                }
                let mut stream_participants = stream_participants;
                let captured_pg_enc = if let Some(ref pool) = pg_pool_opt {
                    Some(spine_api::backup::capture_encrypted_pg_dump(pool).await?)
                } else {
                    None
                };
                if let Some(ref ct) = cancel_token_work {
                    if ct.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(spine_api::core::error::ForgeError::ServiceUnavailable(
                            "Backup aborted: database maintenance barrier lock or connection lost".into(),
                        ));
                    }
                }
                if let Some(client) = coord_client {
                    let snap_id = uuid::Uuid::new_v4().to_string();
                    let (manifest, data) = client
                        .capture_snapshot(&snap_id, fencing_generation)
                        .await
                        .map_err(|e| {
                            spine_api::core::error::ForgeError::Internal(format!(
                                "Failed to capture coordinator snapshot: {}",
                                e
                            ))
                        })?;
                    let coord_enc = spine_api::backup::EphemeralEncryptedTemp::from_reader(
                        &data[..],
                        spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
                    )?;
                    drop(data);
                    stream_participants.push((
                        spine_api::BackupParticipantRecord {
                            participant_id: manifest.participant_id,
                            volume_name: manifest.volume_name,
                            snapshot_id: manifest.snapshot_id,
                            fencing_generation: manifest.fencing_generation,
                            byte_length: manifest.total_bytes,
                            sha256_checksum: manifest.sha256_checksum,
                            archive_rel_path: "coordinator/coordinator_snapshot.tar.gz".to_string(),
                        },
                        spine_api::backup::ParticipantPayloadSource::EncryptedTemp(coord_enc),
                    ));
                    println!("Coordinator volume snapshot captured successfully.");
                }

                if let Some(ref ct) = cancel_token_work {
                    if ct.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(spine_api::core::error::ForgeError::ServiceUnavailable(
                            "Backup aborted: database maintenance barrier lock or connection lost".into(),
                        ));
                    }
                }
                for (node_id, client) in &storage_clients {
                    let snap_id = uuid::Uuid::new_v4().to_string();
                    let (manifest, data) = client
                        .capture_snapshot(&snap_id, fencing_generation)
                        .await
                        .map_err(|e| {
                            spine_api::core::error::ForgeError::Internal(format!(
                                "Failed to capture storage node '{}' snapshot: {}",
                                node_id, e
                            ))
                        })?;
                    let node_enc = spine_api::backup::EphemeralEncryptedTemp::from_reader(
                        &data[..],
                        spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
                    )?;
                    drop(data);
                    stream_participants.push((
                        spine_api::BackupParticipantRecord {
                            participant_id: manifest.participant_id,
                            volume_name: manifest.volume_name,
                            snapshot_id: manifest.snapshot_id,
                            fencing_generation: manifest.fencing_generation,
                            byte_length: manifest.total_bytes,
                            sha256_checksum: manifest.sha256_checksum,
                            archive_rel_path: format!("storage_nodes/{}_snapshot.tar.gz", node_id),
                        },
                        spine_api::backup::ParticipantPayloadSource::EncryptedTemp(node_enc),
                    ));
                    println!(
                        "Storage node '{}' volume snapshot captured successfully.",
                        node_id
                    );
                }

                // Capture authoritative SSH host identity snapshot
                let ssh_dir = spine_api::ssh::host_key::resolve_ssh_dir(&storage_root);
                let ssh_identity_opt =
                    spine_api::backup::BackupManager::capture_ssh_identity_snapshot(
                        &ssh_dir,
                        fencing_generation,
                    )?;
                if let Some((ssh_record, ssh_bytes, fingerprint)) = ssh_identity_opt {
                    println!(
                        "Authoritative SSH host identity snapshot captured (fingerprint: {}).",
                        fingerprint
                    );
                    stream_participants.push((
                        ssh_record,
                        spine_api::backup::ParticipantPayloadSource::Memory(ssh_bytes),
                    ));
                } else if prof.requires_persistent_database() {
                    return Err(spine_api::core::error::ForgeError::Internal(format!(
                        "Under deployment profile '{}', SSH host identity ('spine_ssh') is required. No host key found in '{}'.",
                        prof,
                        ssh_dir.display()
                    )));
                }

                // Register PostgreSQL database participant record with bounded streaming AEAD payload
                if let Some(pg_enc) = captured_pg_enc {
                    stream_participants.push((
                        spine_api::BackupParticipantRecord {
                            participant_id: "postgres_data".to_string(),
                            volume_name: "postgres_data".to_string(),
                            snapshot_id: format!("pg_snap_{}", uuid::Uuid::new_v4().simple()),
                            fencing_generation,
                            byte_length: pg_enc.byte_length,
                            sha256_checksum: pg_enc.sha256_checksum.clone(),
                            archive_rel_path: "database/snapshot.jsonl".to_string(),
                        },
                        spine_api::backup::ParticipantPayloadSource::EncryptedTemp(pg_enc),
                    ));
                    println!("PostgreSQL database snapshot participant record registered (bounded AEAD streaming, 0 plaintext disk intermediates).");
                } else if prof.requires_persistent_database() {
                    return Err(spine_api::core::error::ForgeError::Internal(format!(
                        "Under deployment profile '{}', PostgreSQL database snapshot ('postgres_data') is required.",
                        prof
                    )));
                }

                // Register Spine API storage volume participant record with bounded streaming AEAD payload
                if prof.requires_persistent_database() {
                    println!("Capturing Spine storage volume snapshot into bounded AEAD stream (0 plaintext disk intermediates)...");
                    let storage_enc =
                        spine_api::backup::BackupManager::capture_storage_snapshot_encrypted(
                            &storage_root,
                            spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
                        )?;
                    stream_participants.push((
                        spine_api::BackupParticipantRecord {
                            participant_id: "spine_storage".to_string(),
                            volume_name: "spine_storage".to_string(),
                            snapshot_id: format!("storage_snap_{}", uuid::Uuid::new_v4().simple()),
                            fencing_generation,
                            byte_length: storage_enc.byte_length,
                            sha256_checksum: storage_enc.sha256_checksum.clone(),
                            archive_rel_path: "storage/spine_storage_snapshot.tar.gz".to_string(),
                        },
                        spine_api::backup::ParticipantPayloadSource::EncryptedTemp(storage_enc),
                    ));
                    println!(
                        "Spine storage volume participant record registered (bounded AEAD streaming)."
                    );
                }

                if let Some(ref ct) = cancel_token_work {
                    if ct.load(std::sync::atomic::Ordering::SeqCst) {
                        return Err(spine_api::core::error::ForgeError::ServiceUnavailable(
                            "Backup aborted: database maintenance barrier lock or connection lost before archive assembly".into(),
                        ));
                    }
                }

                // Enforce participant registry validation before archive assembly
                let participant_records: Vec<_> =
                    stream_participants.iter().map(|(r, _)| r.clone()).collect();
                spine_api::backup::participants::ParticipantRegistry::validate_participants(
                    &prof,
                    &participant_records,
                    fencing_generation,
                )
                .map_err(|e| {
                    spine_api::core::error::ForgeError::Internal(format!(
                        "Participant validation failed: {}",
                        e
                    ))
                })?;

                println!(
                    "Creating coordinated backup from {} to {}",
                    storage_root.display(),
                    output.display()
                );
                let staged = spine_api::backup::BackupManager::create_cluster_backup_staged(
                    &storage_root,
                    None,
                    stream_participants,
                    Some(fencing_generation),
                    &output,
                    pass.as_deref(),
                    key_version,
                    cancel_token_work.clone(),
                )?;

                let guard_shared_verify = guard_shared_for_publish.clone();
                let manifest = staged
                    .publish_with_verifier(move || async move {
                        if let Some(ref shared) = guard_shared_verify {
                            let guard_lock = shared.lock().await;
                            if let Some(ref guard) = *guard_lock {
                                guard.verify_lock_held().await?;
                            }
                        }
                        Ok(())
                    })
                    .await?;
                println!("Backup archive created successfully:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Total files:   {}", manifest.total_files);
                println!("  Total bytes:   {}", manifest.total_bytes);
                println!("  Status:        {}", manifest.integrity_status);
                    if !manifest.participants.is_empty() {
                        println!(
                            "  Participants:  {} cluster volumes captured",
                            manifest.participants.len()
                        );
                    }
                    Ok(())
                },
                )
                .await?;
                Ok(())
            }
            BackupCommands::Restore {
                input,
                target_storage,
                target_database_url_file,
                passphrase_file,
                profile,
                coordinator_addr,
                storage_node_addrs,
                allow_unencrypted_legacy_restore,
            } => {
                let prof = parse_cli_profile(profile.as_deref())?;

                let target_storage = if target_storage
                    == std::path::Path::new("./.spine-dev-cluster/node-1")
                {
                    if let Ok(r) = std::env::var("SPINE_STORAGE_ROOT")
                        .or_else(|_| std::env::var("STORAGE_ROOT"))
                    {
                        std::path::PathBuf::from(r.trim())
                    } else if prof.requires_persistent_database() {
                        anyhow::bail!(
                            "Under deployment profile '{}', the default development target storage root ('./.spine-dev-cluster/node-1') is strictly forbidden. Configure STORAGE_ROOT or --target-storage.",
                            prof
                        );
                    } else {
                        target_storage
                    }
                } else {
                    target_storage
                };

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

                let allow_legacy = allow_unencrypted_legacy_restore
                    || std::env::var("SPINE_ALLOW_LEGACY_PLAINTEXT_RESTORE")
                        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                        .unwrap_or(false);

                if prof.requires_persistent_database() && !is_encrypted && !allow_legacy {
                    anyhow::bail!(
                        "Under deployment profile '{}', restoring unencrypted plaintext archives is strictly forbidden. Pass an AEAD-encrypted backup archive (.spinebak) or explicitly pass --allow-unencrypted-legacy-restore / set SPINE_ALLOW_LEGACY_PLAINTEXT_RESTORE=1 for migration.",
                        prof
                    );
                }

                if prof.requires_persistent_database() && target_db.is_none() {
                    anyhow::bail!(
                        "Under deployment profile '{}', a target PostgreSQL database URL is required for restore (--target-database-url-file, or TARGET_DATABASE_URL).",
                        prof
                    );
                }

                // Staging restore session and verifying checksums
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

                let fencing_generation = session.manifest.fencing_generation.unwrap_or(1);
                if fencing_generation == 0 {
                    let _ = session.abort("Manifest contains invalid fencing_generation 0");
                    anyhow::bail!("Manifest contains invalid fencing_generation 0 (must be >= 1)");
                }
                if session.manifest.fencing_generation.is_some()
                    || session.manifest.participants.iter().any(|p| {
                        p.participant_id == "coordinator"
                            || p.participant_id == "coordinator_data"
                            || p.participant_id.starts_with("node-")
                            || p.participant_id.starts_with("storage_")
                    })
                {
                    session.journal.target_generation = fencing_generation.to_string();
                    session.journal.persist(session.journal.phase.clone())?;
                }

                // Enforce participant completeness according to deployment profile
                spine_api::backup::participants::ParticipantRegistry::validate_participants(
                    &prof,
                    &session.manifest.participants,
                    fencing_generation,
                )
                .map_err(|e| {
                    let _ = session.abort(&format!("Participant validation failed: {}", e));
                    anyhow::anyhow!("Participant validation failed: {}", e)
                })?;

                // FAIL-SAFE: Validate target database BEFORE modifying repository storage on disk
                if let Some(ref db_url) = target_db {
                    println!(
                        "Connecting and verifying target database: {}...",
                        sanitize_db_url(db_url)
                    );
                    let pool = sqlx::PgPool::connect(db_url).await.map_err(|e| {
                        let _ = session.abort(&format!("Target DB connect failed: {}", e));
                        anyhow::anyhow!("Failed to connect to target PostgreSQL for restore: {}", e)
                    })?;

                    let empty = spine_api::is_database_empty(&pool).await.map_err(|e| {
                        let _ = session.abort(&format!("Failed to inspect target DB state: {}", e));
                        anyhow::anyhow!("Failed to inspect target database state: {}", e)
                    })?;
                    if !empty {
                        let _ = session.abort("Target database is not empty");
                        anyhow::bail!(
                            "Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption."
                        );
                    }
                    pool.close().await;
                }

                let coord_addr_opt = coordinator_addr
                    .or_else(|| std::env::var("SPINE_COORDINATOR_ADDR").ok())
                    .or_else(|| std::env::var("COORDINATOR_ADDR").ok());
                let storage_nodes_str_opt = storage_node_addrs
                    .or_else(|| std::env::var("SPINE_STORAGE_NODES").ok())
                    .or_else(|| std::env::var("STORAGE_NODE_ADDRS").ok())
                    .or_else(|| std::env::var("STORAGE_NODES").ok());

                let mut node_endpoints = std::collections::HashMap::new();
                if let Some(nodes_str) = storage_nodes_str_opt {
                    for part in nodes_str.split(',') {
                        let trimmed = part.trim();
                        if !trimmed.is_empty() {
                            if let Some((id, addr)) = trimmed.split_once('=') {
                                node_endpoints
                                    .insert(id.trim().to_string(), addr.trim().to_string());
                            } else {
                                node_endpoints.insert(
                                    format!("node-{}", node_endpoints.len() + 1),
                                    trimmed.to_string(),
                                );
                            }
                        }
                    }
                }

                // If production profile, coordinator and 3 storage nodes must have endpoints supplied
                if prof.requires_persistent_database() {
                    if coord_addr_opt.is_none() {
                        let _ = session.abort("Missing coordinator address for production restore");
                        anyhow::bail!(
                            "Under deployment profile '{}', a coordinator address is required for restore (--coordinator-addr, or COORDINATOR_ADDR).",
                            prof
                        );
                    }
                    for needed_node in ["node-1", "node-2", "node-3"] {
                        if !node_endpoints.contains_key(needed_node) {
                            let _ = session.abort(&format!(
                                "Missing storage node endpoint for {}",
                                needed_node
                            ));
                            anyhow::bail!(
                                "Under deployment profile '{}', storage node endpoint for '{}' is required (--storage-node-addrs, or STORAGE_NODE_ADDRS).",
                                prof, needed_node
                            );
                        }
                    }
                }

                // ------------------------------------------------------------------------
                // DISTRIBUTED CUTOVER PROTOCOL
                // Phase 1: PREPARE (Stage restore on all 7 participants)
                // ------------------------------------------------------------------------
                println!("=== Phase 1: PREPARE (Staging All Participants in Isolation) ===");

                // 1. Stage PostgreSQL Database in isolated staging database before modifying live state
                let mut staged_db_restore: Option<spine_api::db::StagedDatabaseRestore> = None;
                if let Some(ref db_url) = target_db {
                    if let Some(ref snap_path) = session.db_snapshot_path {
                        println!("Staging database restore into isolated staging before commit decision...");
                        let s_db = spine_api::db::StagedDatabaseRestore::prepare_from_file(
                            db_url,
                            &session.journal.restore_id,
                            &session.journal.target_generation,
                            &session.manifest.backup_id,
                            snap_path,
                        )
                        .await
                        .map_err(|e| {
                            let _ = session.abort(&format!("Database staging failed: {}", e));
                            anyhow::anyhow!("Database staging failed: {}", e)
                        })?;

                        session
                            .journal
                            .record_participant_ack(s_db.acknowledgement.clone())
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to record postgres_data acknowledgement: {}",
                                    e
                                )
                            })?;
                        println!(
                            "Database restore staged successfully: {} tables, {} rows, digest={}",
                            s_db.tables_restored, s_db.rows_restored, s_db.staged_digest
                        );
                        staged_db_restore = Some(s_db);
                    }
                }

                // 2. Stage Local Repository Storage (spine_storage)
                let mut staged_storage_adapter: Option<
                    spine_api::backup::LocalStorageParticipantAdapter,
                > = None;
                if session
                    .manifest
                    .participants
                    .iter()
                    .any(|p| p.participant_id == "spine_storage")
                {
                    if let Some(storage_data) = session
                        .get_participant_snapshot_data("storage/spine_storage_snapshot.tar.gz")
                    {
                        println!(
                            "Staging local repository storage into isolated staging directory..."
                        );
                        let adapter = spine_api::backup::LocalStorageParticipantAdapter::prepare(
                            &target_storage,
                            &session.journal.restore_id,
                            &session.manifest.backup_id,
                            &session.journal.target_generation,
                            &storage_data,
                        )
                        .map_err(|e| {
                            let _ = session.abort(&format!("LocalStorage staging failed: {}", e));
                            anyhow::anyhow!("LocalStorage staging failed: {}", e)
                        })?;
                        session
                            .journal
                            .record_participant_ack(adapter.acknowledgement.clone())
                            .map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to record spine_storage acknowledgement: {}",
                                    e
                                )
                            })?;
                        staged_storage_adapter = Some(adapter);
                    }
                }

                // 3. Stage Coordinator and Storage Node Volumes via typed RPC
                let mut staged_cluster_restores: Vec<(String, u64, spine_api::CoordinatorClient)> =
                    Vec::new();
                let manifest_participants = session.manifest.participants.clone();
                for participant in &manifest_participants {
                    if participant.participant_id == "coordinator"
                        || participant.archive_rel_path.starts_with("coordinator/")
                    {
                        if let Some(ref c_addr) = coord_addr_opt {
                            if let Some(data) =
                                session.get_participant_snapshot_data(&participant.archive_rel_path)
                            {
                                println!(
                                    "Staging volume restore on Spine Coordinator at {}...",
                                    c_addr
                                );
                                let client = spine_api::CoordinatorClient::new(c_addr.clone());
                                let vol_manifest =
                                    spine_cluster::protocol::VolumeSnapshotManifest {
                                        participant_id: participant.participant_id.clone(),
                                        volume_name: participant.volume_name.clone(),
                                        snapshot_id: participant.snapshot_id.clone(),
                                        restore_id: Some(session.journal.restore_id.clone()),
                                        backup_id: Some(session.manifest.backup_id.clone()),
                                        fencing_generation: participant.fencing_generation,
                                        file_count: 0,
                                        total_bytes: participant.byte_length,
                                        sha256_checksum: participant.sha256_checksum.clone(),
                                        created_at_nanos: 0,
                                    };
                                let ack = match client.stage_restore(vol_manifest, data).await {
                                    Ok(ack) => ack,
                                    Err(e) => {
                                        for (r_snap, _, r_client) in &staged_cluster_restores {
                                            let _ = r_client.rollback_restore(r_snap).await;
                                        }
                                        if let Some(ref sdb) = staged_db_restore {
                                            let _ = sdb.rollback().await;
                                        }
                                        let _ = session.abort(&format!(
                                            "Failed to stage volume restore on coordinator: {}",
                                            e
                                        ));
                                        anyhow::bail!(
                                            "Failed to stage volume restore on coordinator: {}",
                                            e
                                        );
                                    }
                                };
                                session
                                    .journal
                                    .record_participant_snapshot(
                                        &participant.participant_id,
                                        &participant.snapshot_id,
                                    )
                                    .map_err(|e| {
                                        anyhow::anyhow!(
                                            "Failed to record coordinator snapshot ID: {}",
                                            e
                                        )
                                    })?;
                                session.journal.record_participant_ack(ack).map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to record coordinator acknowledgement: {}",
                                        e
                                    )
                                })?;
                                staged_cluster_restores.push((
                                    participant.snapshot_id.clone(),
                                    participant.fencing_generation,
                                    client,
                                ));
                            }
                        }
                    } else if let Some(addr) = node_endpoints.get(&participant.participant_id) {
                        if let Some(data) =
                            session.get_participant_snapshot_data(&participant.archive_rel_path)
                        {
                            println!(
                                "Staging volume restore on storage node '{}' at {}...",
                                participant.participant_id, addr
                            );
                            let client = spine_api::CoordinatorClient::new(addr.clone());
                            let vol_manifest = spine_cluster::protocol::VolumeSnapshotManifest {
                                participant_id: participant.participant_id.clone(),
                                volume_name: participant.volume_name.clone(),
                                snapshot_id: participant.snapshot_id.clone(),
                                restore_id: Some(session.journal.restore_id.clone()),
                                backup_id: Some(session.manifest.backup_id.clone()),
                                fencing_generation: participant.fencing_generation,
                                file_count: 0,
                                total_bytes: participant.byte_length,
                                sha256_checksum: participant.sha256_checksum.clone(),
                                created_at_nanos: 0,
                            };
                            let ack = match client.stage_restore(vol_manifest, data).await {
                                Ok(ack) => ack,
                                Err(e) => {
                                    for (r_snap, _, r_client) in &staged_cluster_restores {
                                        let _ = r_client.rollback_restore(r_snap).await;
                                    }
                                    if let Some(ref sdb) = staged_db_restore {
                                        let _ = sdb.rollback().await;
                                    }
                                    let _ = session.abort(&format!(
                                        "Failed to stage volume restore on storage node '{}': {}",
                                        participant.participant_id, e
                                    ));
                                    anyhow::bail!(
                                        "Failed to stage volume restore on storage node '{}': {}",
                                        participant.participant_id,
                                        e
                                    );
                                }
                            };
                            session
                                .journal
                                .record_participant_snapshot(
                                    &participant.participant_id,
                                    &participant.snapshot_id,
                                )
                                .map_err(|e| {
                                    anyhow::anyhow!(
                                        "Failed to record storage node snapshot ID: {}",
                                        e
                                    )
                                })?;
                            session.journal.record_participant_ack(ack).map_err(|e| {
                                anyhow::anyhow!(
                                    "Failed to record storage node acknowledgement: {}",
                                    e
                                )
                            })?;
                            staged_cluster_restores.push((
                                participant.snapshot_id.clone(),
                                participant.fencing_generation,
                                client,
                            ));
                        }
                    }
                }

                // 4. Prepare SSH identity staging if present
                let mut staged_ssh_adapter: Option<spine_api::backup::SshParticipantAdapter> = None;
                if session
                    .manifest
                    .participants
                    .iter()
                    .any(|p| p.participant_id == "spine_ssh")
                {
                    if let Some(data) =
                        session.get_participant_snapshot_data("ssh/ssh_identity_snapshot.tar.gz")
                    {
                        println!("Validating SSH host identity snapshot in isolated staging...");
                        let adapter = spine_api::backup::SshParticipantAdapter::prepare(
                            &target_storage,
                            &session.journal.restore_id,
                            &session.manifest.backup_id,
                            &session.journal.target_generation,
                            &data,
                        )
                        .map_err(|e| {
                            let _ = session
                                .abort(&format!("SSH identity staging validation failed: {}", e));
                            anyhow::anyhow!("SSH identity staging validation failed: {}", e)
                        })?;
                        session
                            .journal
                            .record_participant_ack(adapter.acknowledgement.clone())
                            .map_err(|e| {
                                anyhow::anyhow!("Failed to record spine_ssh acknowledgement: {}", e)
                            })?;
                        staged_ssh_adapter = Some(adapter);
                    }
                }

                // ------------------------------------------------------------------------
                // Phase 2: PREPARED (All Participants Staged and Verified)
                // ------------------------------------------------------------------------
                println!("=== Phase 2: PREPARED (All Participants Staged and Verified) ===");
                let expected_participants: Vec<String> = session
                    .manifest
                    .participants
                    .iter()
                    .map(|p| p.participant_id.clone())
                    .collect();

                if let Err(e) = session
                    .journal
                    .validate_commit_decision_gate(&expected_participants)
                {
                    for (r_snap, _, r_client) in &staged_cluster_restores {
                        let _ = r_client.rollback_restore(r_snap).await;
                    }
                    if let Some(ref sdb) = staged_db_restore {
                        let _ = sdb.rollback().await;
                    }
                    if let Some(ref s_storage) = staged_storage_adapter {
                        let _ = s_storage.rollback();
                    }
                    if let Some(ref s_ssh) = staged_ssh_adapter {
                        let _ = s_ssh.rollback();
                    }
                    let _ = session.abort(&format!("Commit decision gate failed: {}", e));
                    anyhow::bail!("Commit decision gate failed: {}", e);
                }

                session
                    .journal
                    .persist(spine_api::backup::RestorePhase::Prepared)
                    .map_err(|e| {
                        anyhow::anyhow!("Failed to record Prepared phase in journal: {}", e)
                    })?;

                // ------------------------------------------------------------------------
                // Phase 3: COMMIT DECISION (Durable Pre-Cutover Transaction Record)
                // Active Decision Boundary Revalidation across all 7 participants
                // ------------------------------------------------------------------------
                println!("=== Phase 3: COMMIT DECISION (Durable Pre-Cutover Ledger Invariant) ===");
                println!("Actively revalidating durable staging across all participants prior to commit decision...");

                let mut revalidation_failed: Option<String> = None;

                // 1. Re-query PostgreSQL staged database
                if let Some(ref sdb) = staged_db_restore {
                    match sdb.revalidate_staged_database().await {
                        Ok(()) => {
                            tracing::info!(
                                "Decision boundary: PostgreSQL staged database fully revalidated."
                            );
                        }
                        Err(e) => {
                            revalidation_failed = Some(format!(
                                "PostgreSQL staged database revalidation error: {}",
                                e
                            ));
                        }
                    }
                }

                // 2. Re-query local repository storage adapter
                if revalidation_failed.is_none() {
                    if let Some(ref s_storage) = staged_storage_adapter {
                        match s_storage.revalidate_staged_contents() {
                            Ok(()) => {
                                tracing::info!("Decision boundary: Local repository storage contents fully revalidated.");
                            }
                            Err(e) => {
                                revalidation_failed = Some(format!(
                                    "LocalStorage staging content revalidation error: {}",
                                    e
                                ));
                            }
                        }
                    }
                }

                // 3. Re-query SSH identity adapter
                if revalidation_failed.is_none() {
                    if let Some(ref s_ssh) = staged_ssh_adapter {
                        match s_ssh.revalidate_staged_contents() {
                            Ok(()) => {
                                tracing::info!(
                                    "Decision boundary: SSH identity staging contents fully revalidated."
                                );
                            }
                            Err(e) => {
                                revalidation_failed =
                                    Some(format!("SSH staging content revalidation error: {}", e));
                            }
                        }
                    }
                }

                // 4. Re-query Coordinator and Storage Nodes via RPC
                if revalidation_failed.is_none() {
                    for (snap_id, _, client) in &staged_cluster_restores {
                        match client.query_staged_restore(snap_id).await {
                            Ok(Some(ack)) if ack.prepared => {
                                if ack.restore_id != session.journal.restore_id {
                                    revalidation_failed = Some(format!(
                                        "Cluster node restore_id mismatch for snapshot '{}': expected {}, got {}",
                                        snap_id, session.journal.restore_id, ack.restore_id
                                    ));
                                    break;
                                }
                                if ack.backup_id != session.manifest.backup_id {
                                    revalidation_failed = Some(format!(
                                        "Cluster node backup_id mismatch for snapshot '{}': expected {}, got {}",
                                        snap_id, session.manifest.backup_id, ack.backup_id
                                    ));
                                    break;
                                }
                                if ack.target_generation != session.journal.target_generation {
                                    revalidation_failed = Some(format!(
                                        "Cluster node target generation mismatch for snapshot '{}': expected {}, got {}",
                                        snap_id, session.journal.target_generation, ack.target_generation
                                    ));
                                    break;
                                }
                                if ack.staged_digest.trim().is_empty() {
                                    revalidation_failed = Some(format!(
                                        "Cluster node for snapshot '{}' returned empty staged digest",
                                        snap_id
                                    ));
                                    break;
                                }
                                let m = match session.manifest.participants.iter().find(|p| {
                                    p.snapshot_id == *snap_id
                                        && p.participant_id == ack.participant_id
                                }) {
                                    Some(m) => m,
                                    None => {
                                        revalidation_failed = Some(format!(
                                            "Cluster node '{}' with snapshot '{}' not found in manifest participants",
                                            ack.participant_id, snap_id
                                        ));
                                        break;
                                    }
                                };
                                if ack.archive_payload_sha256.trim().is_empty() {
                                    revalidation_failed = Some(format!(
                                        "Cluster node '{}' returned empty archive payload SHA-256",
                                        ack.participant_id
                                    ));
                                    break;
                                }
                                if ack.archive_payload_sha256 != m.sha256_checksum {
                                    revalidation_failed = Some(format!(
                                        "Cluster node '{}' archive payload SHA-256 mismatch: manifest has {}, ack has {}",
                                        ack.participant_id, m.sha256_checksum, ack.archive_payload_sha256
                                    ));
                                    break;
                                }
                                if ack.canonical_resource_id.trim().is_empty() {
                                    revalidation_failed = Some(format!(
                                        "Cluster node '{}' returned empty canonical_resource_id",
                                        ack.participant_id
                                    ));
                                    break;
                                }
                                if ack.durable_staging_record_id.trim().is_empty() {
                                    revalidation_failed = Some(format!(
                                        "Cluster node '{}' returned empty durable_staging_record_id",
                                        ack.participant_id
                                    ));
                                    break;
                                }
                                tracing::info!("Decision boundary: Cluster participant for snapshot '{}' verified prepared across all fields.", snap_id);
                            }
                            Ok(Some(_)) => {
                                revalidation_failed = Some(format!(
                                    "Cluster node for snapshot '{}' is not in prepared state",
                                    snap_id
                                ));
                                break;
                            }
                            Ok(None) => {
                                revalidation_failed = Some(format!("Cluster node for snapshot '{}' returned no staged restore status", snap_id));
                                break;
                            }
                            Err(e) => {
                                revalidation_failed = Some(format!("Cluster node query_staged_restore failed for snapshot '{}': {}", snap_id, e));
                                break;
                            }
                        }
                    }
                }

                if let Some(err_msg) = revalidation_failed {
                    for (r_snap, _, r_client) in &staged_cluster_restores {
                        let _ = r_client.rollback_restore(r_snap).await;
                    }
                    if let Some(ref sdb) = staged_db_restore {
                        let _ = sdb.rollback().await;
                    }
                    if let Some(ref s_storage) = staged_storage_adapter {
                        let _ = s_storage.rollback();
                    }
                    if let Some(ref s_ssh) = staged_ssh_adapter {
                        let _ = s_ssh.rollback();
                    }
                    let _ = session.abort(&format!(
                        "Decision boundary revalidation failed: {}",
                        err_msg
                    ));
                    anyhow::bail!("Decision boundary revalidation failed: {}", err_msg);
                }

                session
                    .journal
                    .validate_commit_decision_gate(&expected_participants)
                    .map_err(|e| {
                        let _ = session.abort(&format!("Commit decision validation failed: {}", e));
                        anyhow::anyhow!("Commit decision validation failed: {}", e)
                    })?;

                let decision_token = format!(
                    "commit_{}_{}",
                    session.manifest.backup_id,
                    uuid::Uuid::new_v4().simple()
                );
                let decision_record = spine_api::backup::CommitDecisionRecord {
                    restore_id: session.journal.restore_id.clone(),
                    backup_id: session.manifest.backup_id.clone(),
                    old_generation: session.journal.previous_storage_generation.clone(),
                    new_generation: session.journal.target_generation.clone(),
                    required_participants: session
                        .manifest
                        .participants
                        .iter()
                        .map(|p| p.participant_id.clone())
                        .collect(),
                    staged_manifest_digests: session.manifest.file_hashes.clone(),
                    decision_timestamp_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
                    schema_identity: session.manifest.schema_version.clone(),
                    decision_token,
                };
                session
                    .journal
                    .persist(spine_api::backup::RestorePhase::CommitDecision(
                        decision_record.clone(),
                    ))
                    .map_err(|e| {
                        anyhow::anyhow!(
                            "Failed to persist CommitDecisionRecord to restore journal: {}",
                            e
                        )
                    })?;
                println!("Durable CommitDecisionRecord logged to journal before any irreversible participant mutations.");

                // ------------------------------------------------------------------------
                // Phase 4: PUBLISH (Executing Distributed Cutover in Strict Order)
                // 1. Database -> 2. Storage Root -> 3. Cluster Volumes -> 4. SSH Host Key
                // ------------------------------------------------------------------------
                println!("=== Phase 4: PUBLISH (Executing Distributed Cutover) ===");

                // Step 1: Database cutover
                if let Some(ref sdb) = staged_db_restore {
                    println!("Atomically cutting over staged database to live target database...");
                    if let Err(e) = sdb.commit_cutover(&target_storage, &decision_record).await {
                        // After CommitDecision, NEVER roll back participants. Fencing remains active.
                        let _ =
                            session
                                .journal
                                .persist(spine_api::backup::RestorePhase::Publishing {
                                    published_participants: session
                                        .journal
                                        .published_participants
                                        .clone(),
                                });
                        anyhow::bail!(
                            "FATAL: Database cutover failed after durable CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                            e
                        );
                    }
                    session
                        .journal
                        .record_participant_published("postgres_data")
                        .map_err(|e| anyhow::anyhow!("Failed to update restore journal: {}", e))?;
                    println!("Database cutover completed successfully.");
                }

                // Step 2: Publish verified repository storage to live root (retaining rollback generation)
                println!("Atomically publishing verified repository storage to live root (retaining rollback generation)...");
                let manifest = session.manifest.clone();
                let report = if let Some(ref s_storage) = staged_storage_adapter {
                    if let Err(e) = s_storage.publish() {
                        // After CommitDecision, NEVER roll back participants. Fencing remains active.
                        let _ =
                            session
                                .journal
                                .persist(spine_api::backup::RestorePhase::Publishing {
                                    published_participants: session
                                        .journal
                                        .published_participants
                                        .clone(),
                                });
                        anyhow::bail!(
                            "FATAL: Storage publication failed after durable CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                            e
                        );
                    }
                    let _ = session
                        .journal
                        .record_participant_published("spine_storage");
                    spine_api::backup::RestoreReport {
                        backup_id: manifest.backup_id.clone(),
                        files_restored: 1,
                        bytes_restored: s_storage.staged_byte_length,
                        integrity_verified: true,
                        duration_ms: 0,
                    }
                } else {
                    match session.publish_storage() {
                        Ok(r) => {
                            let _ = session
                                .journal
                                .record_participant_published("spine_storage");
                            r
                        }
                        Err(e) => {
                            // After CommitDecision, NEVER roll back participants. Fencing remains active.
                            let _ = session.journal.persist(
                                spine_api::backup::RestorePhase::Publishing {
                                    published_participants: session
                                        .journal
                                        .published_participants
                                        .clone(),
                                },
                            );
                            anyhow::bail!(
                                "FATAL: Storage publication failed after durable CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                                e
                            );
                        }
                    }
                };

                // Step 3: Commit cluster volume participants
                println!("Committing volume restores on cluster participants...");
                let mut committed_cluster_restores: Vec<(
                    String,
                    u64,
                    spine_api::CoordinatorClient,
                )> = Vec::new();
                for (snap_id, fencing_gen, client) in &staged_cluster_restores {
                    println!("Committing volume restore for snapshot {}...", snap_id);
                    if let Err(e) = client.commit_restore(snap_id, *fencing_gen).await {
                        // After CommitDecision, NEVER roll back participants. Fencing remains active.
                        let _ =
                            session
                                .journal
                                .persist(spine_api::backup::RestorePhase::Publishing {
                                    published_participants: session
                                        .journal
                                        .published_participants
                                        .clone(),
                                });
                        anyhow::bail!(
                            "FATAL: Cluster volume commit for snapshot '{}' failed after durable CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                            snap_id,
                            e
                        );
                    }
                    let _ = session.journal.record_participant_published(snap_id);
                    committed_cluster_restores.push((
                        snap_id.clone(),
                        *fencing_gen,
                        client.clone(),
                    ));
                }

                // Step 4: Publish staged SSH host identity with rollback preservation
                if let Some(ref s_ssh) = staged_ssh_adapter {
                    println!("Publishing staged SSH host identity...");
                    if let Err(e) = s_ssh.publish() {
                        // After CommitDecision, NEVER roll back participants. Fencing remains active.
                        let _ =
                            session
                                .journal
                                .persist(spine_api::backup::RestorePhase::Publishing {
                                    published_participants: session
                                        .journal
                                        .published_participants
                                        .clone(),
                                });
                        anyhow::bail!(
                            "FATAL: SSH identity publish failed after durable CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                            e
                        );
                    }
                    let _ = session.journal.record_participant_published("spine_ssh");
                    println!("SSH host identity published successfully.");
                }

                session
                    .journal
                    .persist(spine_api::backup::RestorePhase::Published)
                    .map_err(|e| anyhow::anyhow!("Failed to record Published phase: {}", e))?;

                // ------------------------------------------------------------------------
                // Phase 5: VERIFY
                // Confirm health, fencing generation, and integrity across all participants
                // ------------------------------------------------------------------------
                println!("=== Phase 5: VERIFY (Post-Cutover Distributed Integrity Validation) ===");
                let mut verification_failed = false;
                let mut verify_err_msg = String::new();

                for (snap_id, expected_gen, client) in &committed_cluster_restores {
                    match client.query_maintenance_status().await {
                        Ok((_, current_gen)) => {
                            if current_gen < *expected_gen {
                                verification_failed = true;
                                verify_err_msg = format!("Participant for snapshot {} reported generation {} < expected {}", snap_id, current_gen, expected_gen);
                                break;
                            }
                        }
                        Err(e) => {
                            verification_failed = true;
                            verify_err_msg = format!(
                                "Failed to verify participant for snapshot {}: {}",
                                snap_id, e
                            );
                            break;
                        }
                    }
                }

                if verification_failed {
                    println!(
                        "Post-cutover verification failed: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                        verify_err_msg
                    );
                    anyhow::bail!(
                        "FATAL: Restore failed post-cutover verification after CommitDecision: {}. Writes remain fenced; restart recovery will resume forward convergence.",
                        verify_err_msg
                    );
                }

                session
                    .journal
                    .persist(spine_api::backup::RestorePhase::Verified)
                    .map_err(|e| anyhow::anyhow!("Failed to record Verified phase: {}", e))?;
                println!("All participants verified against committed target generation.");

                // ------------------------------------------------------------------------
                // Phase 6: FINALIZE
                // Purge .old_restore, .old_generation, and .old_ssh across all participants
                // ------------------------------------------------------------------------
                println!("=== Phase 6: FINALIZE (Purging Retained Old Generations) ===");
                if let Some(ref s_ssh) = staged_ssh_adapter {
                    s_ssh.finalize().map_err(|e| {
                        anyhow::anyhow!(
                            "FATAL: Failed to finalize SSH participant after CommitDecision: {}. Writes remain fenced; restart recovery will complete finalization.",
                            e
                        )
                    })?;
                    let _ = session.journal.record_participant_recovery(
                        "spine_ssh",
                        true,
                        true,
                        Some(&session.journal.target_generation.clone()),
                    );
                }
                if let Some(ref sdb) = staged_db_restore {
                    sdb.finalize().await.map_err(|e| {
                        anyhow::anyhow!(
                            "FATAL: Failed to finalize PostgreSQL database restore after CommitDecision: {}. Writes remain fenced; restart recovery will complete finalization.",
                            e
                        )
                    })?;
                    let _ = session.journal.record_participant_recovery(
                        "postgres_data",
                        true,
                        true,
                        Some(&session.journal.target_generation.clone()),
                    );
                }
                if let Some(ref s_storage) = staged_storage_adapter {
                    s_storage.finalize().map_err(|e| {
                        anyhow::anyhow!(
                            "FATAL: Failed to finalize local storage after CommitDecision: {}. Writes remain fenced; restart recovery will complete finalization.",
                            e
                        )
                    })?;
                    let _ = session.journal.record_participant_recovery(
                        "spine_storage",
                        true,
                        true,
                        Some(&session.journal.target_generation.clone()),
                    );
                }
                for (snap_id, _, client) in &committed_cluster_restores {
                    client.finalize_restore(snap_id).await.map_err(|e| {
                        anyhow::anyhow!(
                            "FATAL: Failed to finalize restore on cluster participant for snapshot '{}': {}. Writes remain fenced; restart recovery will complete finalization.",
                            snap_id,
                            e
                        )
                    })?;
                }
                let finalize_report = session
                    .finalize()
                    .map_err(|e| anyhow::anyhow!("Failed to finalize storage session: {}", e))?;

                println!("Restoration sequence completed successfully:");
                println!("  Backup ID:     {}", manifest.backup_id);
                println!("  Files restored:{}", report.files_restored);
                println!("  Bytes restored:{}", report.bytes_restored);
                println!(
                    "  Integrity:     {}",
                    if finalize_report.integrity_verified {
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
            BackupCommands::Encrypt {
                input,
                output,
                passphrase_file,
                key_version,
            } => {
                let pass = read_secret(
                    None,
                    passphrase_file.as_deref(),
                    &["SPINE_BACKUP_PASSPHRASE", "BACKUP_PASSPHRASE"],
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("Passphrase required for backup encryption (--passphrase-file or SPINE_BACKUP_PASSPHRASE)")
                })?;

                let is_stdin = input == std::path::Path::new("-");
                let is_stdout = output == std::path::Path::new("-");

                let mut reader: Box<dyn std::io::Read> = if is_stdin {
                    Box::new(std::io::stdin())
                } else {
                    let f = std::fs::File::open(&input)
                        .map_err(|e| anyhow::anyhow!("Failed to open input file {}: {}", input.display(), e))?;
                    Box::new(std::io::BufReader::new(f))
                };

                let mut writer: Box<dyn std::io::Write> = if is_stdout {
                    Box::new(std::io::stdout())
                } else {
                    let f = std::fs::File::create(&output)
                        .map_err(|e| anyhow::anyhow!("Failed to create output file {}: {}", output.display(), e))?;
                    Box::new(std::io::BufWriter::new(f))
                };

                eprintln!(
                    "Encrypting {} into SPNEBAK v2 AEAD envelope (AES-256-GCM)...",
                    if is_stdin { "stdin" } else { input.to_str().unwrap_or("file") }
                );
                let summary = spine_api::backup::v2::encrypt_stream(
                    &mut reader,
                    &mut writer,
                    &pass,
                    key_version,
                    spine_api::backup::v2::DEFAULT_CHUNK_SIZE,
                )
                .map_err(|e| anyhow::anyhow!("Encryption failed: {}", e))?;

                eprintln!(
                    "Encryption complete: {} plaintext bytes encrypted into {} chunks (hash: {})",
                    summary.total_plaintext_bytes,
                    summary.total_chunks,
                    hex::encode(summary.stream_sha256)
                );
                Ok(())
            }
            BackupCommands::Decrypt {
                input,
                output,
                passphrase_file,
            } => {
                let pass = read_secret(
                    None,
                    passphrase_file.as_deref(),
                    &["SPINE_BACKUP_PASSPHRASE", "BACKUP_PASSPHRASE"],
                )?
                .ok_or_else(|| {
                    anyhow::anyhow!("Passphrase required for backup decryption (--passphrase-file or SPINE_BACKUP_PASSPHRASE)")
                })?;

                let is_stdin = input == std::path::Path::new("-");
                let is_stdout = output == std::path::Path::new("-");

                let mut reader: Box<dyn std::io::Read> = if is_stdin {
                    Box::new(std::io::stdin())
                } else {
                    let f = std::fs::File::open(&input)
                        .map_err(|e| anyhow::anyhow!("Failed to open input file {}: {}", input.display(), e))?;
                    Box::new(std::io::BufReader::new(f))
                };

                let mut writer: Box<dyn std::io::Write> = if is_stdout {
                    Box::new(std::io::stdout())
                } else {
                    let f = std::fs::File::create(&output)
                        .map_err(|e| anyhow::anyhow!("Failed to create output file {}: {}", output.display(), e))?;
                    Box::new(std::io::BufWriter::new(f))
                };

                eprintln!(
                    "Authenticating and decrypting SPNEBAK v2 archive (AES-256-GCM)..."
                );
                let summary = spine_api::backup::v2::decrypt_stream(
                    &mut reader,
                    &mut writer,
                    &pass,
                )
                .map_err(|e| anyhow::anyhow!("Authentication or decryption failed: {}", e))?;

                eprintln!(
                    "Authentication and decryption verified [OK]: {} plaintext bytes verified in {} chunks (hash: {})",
                    summary.total_plaintext_bytes,
                    summary.total_chunks,
                    hex::encode(summary.stream_sha256)
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

    let bind = std::env::var("BIND")
        .or_else(|_| std::env::var("SPINE_BIND"))
        .unwrap_or(args.bind);
    let port = std::env::var("PORT")
        .or_else(|_| std::env::var("SPINE_PORT"))
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(args.port);
    let ssh_port = std::env::var("SSH_PORT")
        .or_else(|_| std::env::var("SPINE_SSH_PORT"))
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(args.ssh_port);

    let database_url = resolve_postgres_url(
        None,
        args.database_url_file.as_deref(),
        &["DATABASE_URL", "SPINE_DATABASE_URL"],
        Some("POSTGRES_APP_USER"),
        Some("POSTGRES_APP_PASSWORD"),
        Some("POSTGRES_APP_PASSWORD_FILE"),
    )?;

    let migration_database_url = resolve_postgres_url(
        None,
        args.migration_database_url_file.as_deref(),
        &["MIGRATION_DATABASE_URL", "SPINE_MIGRATION_DATABASE_URL"],
        Some("POSTGRES_MIGRATOR_USER"),
        Some("POSTGRES_MIGRATOR_PASSWORD"),
        Some("POSTGRES_MIGRATOR_PASSWORD_FILE"),
    )?;

    let coordinator_addr = args
        .coordinator_addr
        .or_else(|| std::env::var("SPINE_COORDINATOR_ADDR").ok())
        .or_else(|| std::env::var("COORDINATOR_ADDR").ok());

    let raw_profile = args
        .profile
        .or_else(|| std::env::var("SPINE_DEPLOYMENT_PROFILE").ok())
        .unwrap_or_else(|| "core-forge-v1".to_string());
    let profile = spine_api::DeploymentProfile::from_str_relaxed(&raw_profile)
        .map_err(|e| anyhow::anyhow!(e))?;

    if profile.requires_persistent_database() && coordinator_addr.is_none() {
        anyhow::bail!(
            "CRITICAL CONFIGURATION ERROR: Under deployment profile '{}', a coordinator address is required. Set COORDINATOR_ADDR or --coordinator-addr.",
            profile
        );
    }

    let storage_nodes_str_opt = std::env::var("SPINE_STORAGE_NODES")
        .or_else(|_| std::env::var("STORAGE_NODE_ADDRS"))
        .or_else(|_| std::env::var("STORAGE_NODES"))
        .ok();

    let mut storage_nodes = Vec::new();
    if let Some(ref s) = storage_nodes_str_opt {
        for part in s.split(',') {
            let trimmed = part.trim();
            if !trimmed.is_empty() {
                if let Some((id, addr)) = trimmed.split_once('=') {
                    storage_nodes.push((id.trim().to_string(), addr.trim().to_string()));
                } else {
                    storage_nodes.push((
                        format!("node-{}", storage_nodes.len() + 1),
                        trimmed.to_string(),
                    ));
                }
            }
        }
    }

    if profile.requires_persistent_database() && storage_nodes.len() < 3 {
        anyhow::bail!(
            "CRITICAL CONFIGURATION ERROR: Under deployment profile '{}', all 3 storage nodes (node-1, node-2, node-3) must be configured in SPINE_STORAGE_NODES or STORAGE_NODE_ADDRS. Found {} nodes.",
            profile,
            storage_nodes.len()
        );
    }

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

    // 1. Resolve and ensure Storage Root exists
    let storage_root_path = if let Ok(root_str) =
        std::env::var("SPINE_STORAGE_ROOT").or_else(|_| std::env::var("STORAGE_ROOT"))
    {
        std::path::PathBuf::from(root_str.trim())
    } else {
        args.storage_root
    };
    if profile.requires_persistent_database()
        && storage_root_path == std::path::Path::new("./.spine-dev-cluster/node-1")
    {
        anyhow::bail!(
            "CRITICAL CONFIGURATION ERROR: Production profile '{}' disallows default development storage root './.spine-dev-cluster/node-1'. Configure SPINE_STORAGE_ROOT, STORAGE_ROOT, or --storage-root.",
            profile
        );
    }
    std::fs::create_dir_all(&storage_root_path)?;
    let storage_root_canonical = std::fs::canonicalize(&storage_root_path)?;
    tracing::info!("Storage root: {}", storage_root_canonical.display());

    // Resolve authoritative active database URLs using DatabaseSelector (Fail closed, no fallback)
    let database_url = match database_url {
        Some(raw_url) => {
            let active_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                &raw_url,
                Some(&storage_root_canonical),
            )
            .await?;
            if active_url != raw_url {
                tracing::info!(
                    "DatabaseSelector resolved active database URL: {}",
                    sanitize_db_url(&active_url)
                );
            }
            Some(active_url)
        }
        None => None,
    };
    let migration_database_url = match migration_database_url {
        Some(raw_mig_url) => {
            let active_mig_url = spine_api::db::DatabaseSelector::resolve_active_db_url(
                &raw_mig_url,
                Some(&storage_root_canonical),
            )
            .await?;
            Some(active_mig_url)
        }
        None => None,
    };

    // 2. Initialize Coordinator Client
    let coordinator = coordinator_addr.map(|addr| {
        tracing::info!("Configured Spine Coordinator at {}", addr);
        CoordinatorClient::new(addr)
    });

    // 2.1 Restore state machine crash recovery & generation validation BEFORE running schema migrations
    let recovery_pool = match database_url.as_ref() {
        Some(url) => Some(PgDatabase::build_pool(url).await.map_err(|e| {
            anyhow::anyhow!(
                "Failed to connect to active database for recovery verification: {}",
                e
            )
        })?),
        None => None,
    };

    if let Some(msg) = spine_api::backup::RestoreJournal::handle_startup_recovery_cluster(
        &storage_root_canonical,
        recovery_pool.as_ref(),
        coordinator.as_ref(),
        &storage_nodes,
    )
    .await
    .map_err(|e| anyhow::anyhow!("FATAL RESTORE RECOVERY FAILURE: {}", e))?
    {
        tracing::warn!("Restore state machine recovery: {}", msg);
    }
    if spine_api::backup::reconcile_backup_maintenance_on_startup(
        &storage_root_canonical,
        recovery_pool.as_ref(),
    )
    .await
    .map_err(|e| anyhow::anyhow!("FATAL BACKUP MAINTENANCE RECOVERY FAILURE: {}", e))?
    {
        tracing::warn!("Startup reconciled stale backup maintenance lease");
    }
    if let Some(p) = recovery_pool {
        p.close().await;
    }

    // 3. Initialize Database backend & apply migrations on verified cluster state
    let (db, pg_pool): (Arc<dyn Database>, Option<sqlx::PgPool>) = match database_url {
        Some(db_url) => {
            if let Some(mig_url) = migration_database_url {
                tracing::info!(
                    "Running baseline schema migrations using dedicated migration credentials ({}) under profile '{}'...",
                    sanitize_db_url(&mig_url),
                    profile
                );
                let migrator_pool = PgDatabase::build_pool(&mig_url).await.map_err(|e| {
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
        Some(pool) => spine_api::backup::ConsistencyBarrier::with_pool(pool)
            .with_storage_root(storage_root_canonical.clone()),
        None => spine_api::backup::ConsistencyBarrier::new()
            .with_storage_root(storage_root_canonical.clone()),
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
                format!("http://{}:{}", bind, port)
            }
        });

    validate_public_origin(&public_origin, profile)?;
    spine_api::auth::validate_production_secrets(profile)?;

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
        webhook_worker: Arc::clone(&webhook_worker),
        workflows,
        scheduler: Arc::clone(&scheduler),
        runners,
        runner_executor,
        search,
        discussions,
        discovery,
        security,
        public_origin,
        consistency_barrier: Arc::clone(&consistency_barrier),
        storage_nodes,
        active_maintenance: Arc::new(tokio::sync::Mutex::new(None)),
    };

    // 4. Spawn background workers
    let worker = Arc::new(
        ProvisioningWorker::new(Arc::clone(&db), coordinator, &storage_root_canonical)
            .with_barrier(Arc::clone(&consistency_barrier)),
    );
    let worker_clone = Arc::clone(&worker);
    let worker_handle = tokio::spawn(async move {
        tracing::info!("Starting background repository provisioning worker");
        worker_clone.run().await;
    });

    let webhook_worker_clone = Arc::clone(&webhook_worker);
    let webhook_worker_handle = tokio::spawn(async move {
        tracing::info!("Starting background webhook outbox delivery worker");
        webhook_worker_clone.run().await;
    });

    let scheduler_clone = Arc::clone(&scheduler);
    let scheduler_handle = tokio::spawn(async move {
        tracing::info!("Starting background workflow scheduler worker");
        scheduler_clone.run().await;
    });

    // 5. Start HTTP and SSH servers
    let ssh_addr_str =
        std::env::var("SSH_ADDR").unwrap_or_else(|_| format!("{}:{}", bind, ssh_port));
    let ssh_addr: SocketAddr = ssh_addr_str.parse()?;
    let (_bound_ssh, _ssh_handle) = spine_api::start_ssh_server(state.clone(), ssh_addr).await?;

    let app = build_router(state);
    let addr: SocketAddr = format!("{}:{}", bind, port).parse()?;
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
    webhook_worker.shutdown();
    scheduler.shutdown();
    let _ = tokio::join!(worker_handle, webhook_worker_handle, scheduler_handle);
    tracing::info!("Spine Forge shutdown complete.");
    Ok(())
}
