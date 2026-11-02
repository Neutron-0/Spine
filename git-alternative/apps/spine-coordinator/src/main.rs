pub mod server;

use anyhow::Result;
use clap::{Parser, Subcommand};
use spine_cluster::enrollment::EnrollmentToken;
use spine_cluster::store::{CoordinatorStore, FileBackedCoordinatorStore};
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "spine-coordinator",
    about = "Spine Distributed Storage Control Plane Daemon"
)]
struct Cli {
    #[arg(long, default_value = "spine-cluster-1")]
    cluster_id: String,

    #[arg(short, long, default_value = "127.0.0.1:9000")]
    listen: String,

    #[arg(long, default_value = ".spine-coordinator-state.json")]
    store_file: PathBuf,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Generate a single-use enrollment token for a new storage node
    GenerateToken {
        #[arg(short, long, default_value_t = 3600)] // 1 hour
        duration_secs: i64,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Arc::new(FileBackedCoordinatorStore::new_file_backed(
        &cli.store_file,
    )?);

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::GenerateToken { duration_secs } => {
                let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                let token = EnrollmentToken::generate(&cli.cluster_id, duration_secs, now);
                store.create_token(token.clone())?;
                println!("Generated single-use enrollment token:");
                println!("Token:      {}", token.token);
                println!("Cluster ID: {}", token.cluster_id);
                println!("Expires In: {} seconds", duration_secs);
            }
        }
        return Ok(());
    }

    println!("===========================================================");
    println!("      SPINE DISTRIBUTED COORDINATOR DAEMON                ");
    println!("===========================================================");
    println!("Cluster ID:   {}", cli.cluster_id);
    println!("Listen Addr:  {}", cli.listen);
    println!("Store Path:   {}", cli.store_file.display());
    println!("===========================================================");

    let server = server::CoordinatorServer::new(cli.cluster_id, cli.listen, store);
    server.run().await?;
    Ok(())
}
