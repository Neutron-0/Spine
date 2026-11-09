pub mod server;

use anyhow::{Context, Result};
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
pub struct Cli {
    #[arg(long, default_value = "spine-cluster-1")]
    pub cluster_id: String,

    #[arg(short, long, default_value = "127.0.0.1:9000")]
    pub listen: String,

    #[arg(long, default_value = ".spine-coordinator-state.json")]
    pub store_file: PathBuf,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Generate a single-use enrollment token for a new storage node
    GenerateToken {
        #[arg(short, long, default_value_t = 3600)] // 1 hour
        duration_secs: i64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoordinatorConfig {
    pub cluster_id: String,
    pub listen_addr: String,
    pub store_file: PathBuf,
}

impl CoordinatorConfig {
    pub fn resolve(cli: &Cli) -> Result<Self> {
        let cluster_id = std::env::var("SPINE_CLUSTER_ID")
            .or_else(|_| std::env::var("CLUSTER_ID"))
            .unwrap_or_else(|_| cli.cluster_id.clone());

        let port_env = std::env::var("SPINE_COORDINATOR_PORT")
            .or_else(|_| std::env::var("COORDINATOR_PORT"))
            .or_else(|_| std::env::var("PORT"))
            .ok();

        let bind_env = std::env::var("SPINE_BIND")
            .or_else(|_| std::env::var("BIND"))
            .or_else(|_| std::env::var("COORDINATOR_BIND"))
            .ok();

        let listen_addr = if let Ok(listen) = std::env::var("SPINE_LISTEN").or_else(|_| std::env::var("COORDINATOR_LISTEN")) {
            listen.trim().to_string()
        } else if let Some(port) = port_env {
            let bind = bind_env.unwrap_or_else(|| "0.0.0.0".to_string());
            format!("{}:{}", bind.trim(), port.trim())
        } else if cli.listen != "127.0.0.1:9000" {
            cli.listen.clone()
        } else if let Some(bind) = bind_env {
            format!("{}:9000", bind.trim())
        } else {
            cli.listen.clone()
        };

        let store_file = if let Ok(path_str) = std::env::var("SPINE_STORE_PATH")
            .or_else(|_| std::env::var("COORDINATOR_STORE_PATH"))
            .or_else(|_| std::env::var("STORE_PATH"))
        {
            PathBuf::from(path_str.trim())
        } else {
            cli.store_file.clone()
        };

        // Validation
        if cluster_id.trim().is_empty() {
            anyhow::bail!("Coordinator cluster_id cannot be empty");
        }
        if listen_addr.trim().is_empty() {
            anyhow::bail!("Coordinator listen_addr cannot be empty");
        }
        if let Some(parent) = store_file.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create coordinator store directory: {}",
                        parent.display()
                    )
                })?;
            }
        }

        Ok(Self {
            cluster_id,
            listen_addr,
            store_file,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = CoordinatorConfig::resolve(&cli)?;

    let store = Arc::new(FileBackedCoordinatorStore::new_file_backed(
        &config.store_file,
    )?);

    if let Some(cmd) = cli.command {
        match cmd {
            Commands::GenerateToken { duration_secs } => {
                let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
                let token = EnrollmentToken::generate(&config.cluster_id, duration_secs, now);
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
    println!("Cluster ID:   {}", config.cluster_id);
    println!("Listen Addr:  {}", config.listen_addr);
    println!("Store Path:   {}", config.store_file.display());
    println!("===========================================================");

    let server = server::CoordinatorServer::new(config.cluster_id, config.listen_addr, store);
    server.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_coordinator_config_resolution_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("SPINE_COORDINATOR_PORT");
        std::env::remove_var("SPINE_BIND");
        std::env::remove_var("SPINE_STORE_PATH");
        std::env::remove_var("SPINE_CLUSTER_ID");

        let cli = Cli {
            cluster_id: "test-cluster".to_string(),
            listen: "127.0.0.1:9000".to_string(),
            store_file: PathBuf::from("test-state.json"),
            command: None,
        };
        let cfg = CoordinatorConfig::resolve(&cli).unwrap();
        assert_eq!(cfg.cluster_id, "test-cluster");
        assert_eq!(cfg.listen_addr, "127.0.0.1:9000");
        assert_eq!(cfg.store_file, PathBuf::from("test-state.json"));
    }

    #[test]
    fn test_coordinator_config_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPINE_COORDINATOR_PORT", "8400");
        std::env::set_var("SPINE_BIND", "0.0.0.0");
        std::env::set_var("SPINE_STORE_PATH", "/var/lib/spine/coordinator.state");
        std::env::set_var("SPINE_CLUSTER_ID", "prod-cluster");

        let cli = Cli {
            cluster_id: "dev-cluster".to_string(),
            listen: "127.0.0.1:9000".to_string(),
            store_file: PathBuf::from(".dev-state.json"),
            command: None,
        };
        let cfg = CoordinatorConfig::resolve(&cli).unwrap();
        assert_eq!(cfg.cluster_id, "prod-cluster");
        assert_eq!(cfg.listen_addr, "0.0.0.0:8400");
        assert_eq!(cfg.store_file, PathBuf::from("/var/lib/spine/coordinator.state"));

        std::env::remove_var("SPINE_COORDINATOR_PORT");
        std::env::remove_var("SPINE_BIND");
        std::env::remove_var("SPINE_STORE_PATH");
        std::env::remove_var("SPINE_CLUSTER_ID");
    }
}
