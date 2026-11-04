use clap::Parser;
use spine_api::{
    auth::{DevMailCatcher, RateLimiter},
    build_router, AppState, ArtifactService, CargoRegistryAdapter, CoordinatorClient, Database,
    InMemoryDatabase, NativeVcsService, PackageService, PermissionService, PgDatabase,
    ProvisioningWorker, ReleaseService, RepositoryService,
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
struct ServerArgs {
    /// Bind IP address
    #[arg(short, long, default_value = "127.0.0.1")]
    bind: String,

    /// Listening port
    #[arg(short, long, default_value_t = 3000)]
    port: u16,

    /// PostgreSQL Database URL (e.g. postgres://spine:spine@127.0.0.1:5432/spine_forge)
    #[arg(long)]
    database_url: Option<String>,

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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "spine_api=info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = ServerArgs::parse();

    let database_url = args
        .database_url
        .or_else(|| std::env::var("DATABASE_URL").ok());
    let coordinator_addr = args
        .coordinator_addr
        .or_else(|| std::env::var("COORDINATOR_ADDR").ok());

    // 1. Initialize Database backend
    let db: Arc<dyn Database> = if args.in_memory_db || database_url.is_none() {
        tracing::info!("Initializing Spine Forge with InMemoryDatabase adapter");
        Arc::new(InMemoryDatabase::new())
    } else {
        let db_url = database_url.unwrap();
        tracing::info!("Connecting to PostgreSQL database...");
        Arc::new(PgDatabase::connect(&db_url).await?)
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
    let artifact_store = Arc::new(spine_artifact::LocalArtifactStore::new(artifact_store_path)?);
    let artifacts = Arc::new(ArtifactService::new(Arc::clone(&db), artifact_store));
    let releases = Arc::new(ReleaseService::new(Arc::clone(&db), Arc::clone(&artifacts)));
    let packages = Arc::new(PackageService::new(Arc::clone(&db), Arc::clone(&artifacts)));
    let cargo = Arc::new(CargoRegistryAdapter::new(Arc::clone(&packages)));

    let state = AppState {
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

    // 5. Start HTTP server
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
