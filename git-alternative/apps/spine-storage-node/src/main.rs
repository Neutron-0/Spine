pub mod backend;
pub mod server;

use anyhow::{Context, Result};
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "spine-storage-node",
    about = "Spine Distributed Storage Node Daemon"
)]
pub struct Args {
    #[arg(short, long, default_value = "node-1")]
    pub node_id: String,

    #[arg(short, long, default_value = "127.0.0.1:9001")]
    pub listen: String,

    #[arg(long, default_value = ".spine-node-data")]
    pub storage_root: PathBuf,

    #[arg(short, long)]
    pub coordinator: Option<String>,

    #[arg(long, default_value_t = 53687091200)] // 50 GB
    pub capacity: u64,

    #[arg(long, default_value = "local")]
    pub region: String,

    #[arg(long, default_value = "zone-1")]
    pub zone: String,

    #[arg(long, default_value = "localhost")]
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageNodeConfig {
    pub node_id: String,
    pub listen_addr: String,
    pub storage_root: PathBuf,
    pub coordinator_addr: Option<String>,
    pub capacity_bytes: u64,
    pub region: String,
    pub zone: String,
    pub host: String,
}

impl StorageNodeConfig {
    pub fn resolve(args: &Args) -> Result<Self> {
        let node_id = std::env::var("SPINE_NODE_ID")
            .or_else(|_| std::env::var("NODE_ID"))
            .unwrap_or_else(|_| args.node_id.clone());

        let port_env = std::env::var("SPINE_PORT")
            .or_else(|_| std::env::var("STORAGE_NODE_PORT"))
            .or_else(|_| std::env::var("PORT"))
            .ok();

        let bind_env = std::env::var("SPINE_BIND")
            .or_else(|_| std::env::var("BIND"))
            .or_else(|_| std::env::var("STORAGE_NODE_BIND"))
            .ok();

        let listen_addr = if let Ok(listen) =
            std::env::var("SPINE_LISTEN").or_else(|_| std::env::var("STORAGE_NODE_LISTEN"))
        {
            listen.trim().to_string()
        } else if let Some(port) = port_env {
            let bind = bind_env.unwrap_or_else(|| "0.0.0.0".to_string());
            format!("{}:{}", bind.trim(), port.trim())
        } else if args.listen != "127.0.0.1:9001" {
            args.listen.clone()
        } else if let Some(bind) = bind_env {
            format!("{}:9001", bind.trim())
        } else {
            args.listen.clone()
        };

        let storage_root = if let Ok(path_str) = std::env::var("SPINE_DATA_DIR")
            .or_else(|_| std::env::var("STORAGE_DATA_DIR"))
            .or_else(|_| std::env::var("SPINE_STORAGE_ROOT"))
            .or_else(|_| std::env::var("STORAGE_ROOT"))
        {
            PathBuf::from(path_str.trim())
        } else {
            args.storage_root.clone()
        };

        let coordinator_addr = std::env::var("SPINE_COORDINATOR_ADDR")
            .or_else(|_| std::env::var("COORDINATOR_ADDR"))
            .ok()
            .or_else(|| args.coordinator.clone());

        let capacity_bytes = if let Ok(cap_str) = std::env::var("SPINE_CAPACITY_BYTES")
            .or_else(|_| std::env::var("STORAGE_CAPACITY_BYTES"))
            .or_else(|_| std::env::var("CAPACITY_BYTES"))
        {
            cap_str
                .trim()
                .parse::<u64>()
                .map_err(|e| anyhow::anyhow!("Invalid capacity '{}': {}", cap_str, e))?
        } else {
            args.capacity
        };

        let zone = std::env::var("SPINE_ZONE")
            .or_else(|_| std::env::var("ZONE"))
            .unwrap_or_else(|_| args.zone.clone());

        let region = std::env::var("SPINE_REGION")
            .or_else(|_| std::env::var("REGION"))
            .unwrap_or_else(|_| args.region.clone());

        let host = std::env::var("SPINE_HOST")
            .or_else(|_| std::env::var("HOST"))
            .unwrap_or_else(|_| args.host.clone());

        // Validation
        if node_id.trim().is_empty() {
            anyhow::bail!("Storage node node_id cannot be empty");
        }
        if listen_addr.trim().is_empty() {
            anyhow::bail!("Storage node listen_addr cannot be empty");
        }
        if !storage_root.exists() {
            std::fs::create_dir_all(&storage_root).with_context(|| {
                format!(
                    "Failed to create storage root directory: {}",
                    storage_root.display()
                )
            })?;
        }

        Ok(Self {
            node_id,
            listen_addr,
            storage_root,
            coordinator_addr,
            capacity_bytes,
            region,
            zone,
            host,
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let config = StorageNodeConfig::resolve(&args)?;

    println!("===========================================================");
    println!("      SPINE DISTRIBUTED STORAGE NODE DAEMON               ");
    println!("===========================================================");
    println!("Node ID:      {}", config.node_id);
    println!("Listen Addr:  {}", config.listen_addr);
    println!("Storage Root: {}", config.storage_root.display());
    println!("Capacity:     {} bytes", config.capacity_bytes);
    println!(
        "Domain:       {}/{}/{}",
        config.region, config.zone, config.host
    );
    if let Some(ref coord) = config.coordinator_addr {
        println!("Coordinator:  {}", coord);
    }
    println!("===========================================================");

    let backend = Arc::new(
        backend::FileSystemStorageBackend::new(&config.storage_root, config.capacity_bytes)?
            .with_node_id(config.node_id.clone()),
    );

    let server = server::StorageNodeServer::new(
        config.node_id,
        config.listen_addr,
        config.coordinator_addr,
        backend,
    );

    server.run().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn test_storage_node_config_resolution_defaults() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::remove_var("SPINE_NODE_ID");
        std::env::remove_var("SPINE_PORT");
        std::env::remove_var("SPINE_BIND");
        std::env::remove_var("SPINE_DATA_DIR");
        std::env::remove_var("SPINE_COORDINATOR_ADDR");
        std::env::remove_var("SPINE_CAPACITY_BYTES");
        std::env::remove_var("SPINE_ZONE");

        let args = Args {
            node_id: "test-node".to_string(),
            listen: "127.0.0.1:9001".to_string(),
            storage_root: PathBuf::from("target/test_node_data"),
            coordinator: None,
            capacity: 50_000_000_000,
            region: "local".to_string(),
            zone: "zone-1".to_string(),
            host: "localhost".to_string(),
        };
        let cfg = StorageNodeConfig::resolve(&args).unwrap();
        assert_eq!(cfg.node_id, "test-node");
        assert_eq!(cfg.listen_addr, "127.0.0.1:9001");
        assert_eq!(cfg.storage_root, PathBuf::from("target/test_node_data"));
        assert!(cfg.coordinator_addr.is_none());
        assert_eq!(cfg.capacity_bytes, 50_000_000_000);
    }

    #[test]
    fn test_storage_node_config_env_overrides() {
        let _guard = ENV_LOCK.lock().unwrap();
        std::env::set_var("SPINE_NODE_ID", "node-3");
        std::env::set_var("SPINE_PORT", "8401");
        std::env::set_var("SPINE_BIND", "0.0.0.0");
        std::env::set_var("SPINE_DATA_DIR", "target/test_node_data_env");
        std::env::set_var("SPINE_COORDINATOR_ADDR", "coordinator:8400");
        std::env::set_var("SPINE_CAPACITY_BYTES", "107374182400");
        std::env::set_var("SPINE_ZONE", "zone-c");

        let args = Args {
            node_id: "dev-node".to_string(),
            listen: "127.0.0.1:9001".to_string(),
            storage_root: PathBuf::from(".dev-data"),
            coordinator: None,
            capacity: 50_000_000_000,
            region: "local".to_string(),
            zone: "zone-1".to_string(),
            host: "localhost".to_string(),
        };
        let cfg = StorageNodeConfig::resolve(&args).unwrap();
        assert_eq!(cfg.node_id, "node-3");
        assert_eq!(cfg.listen_addr, "0.0.0.0:8401");
        assert_eq!(cfg.storage_root, PathBuf::from("target/test_node_data_env"));
        assert_eq!(cfg.coordinator_addr.as_deref(), Some("coordinator:8400"));
        assert_eq!(cfg.capacity_bytes, 107374182400);
        assert_eq!(cfg.zone, "zone-c");

        std::env::remove_var("SPINE_NODE_ID");
        std::env::remove_var("SPINE_PORT");
        std::env::remove_var("SPINE_BIND");
        std::env::remove_var("SPINE_DATA_DIR");
        std::env::remove_var("SPINE_COORDINATOR_ADDR");
        std::env::remove_var("SPINE_CAPACITY_BYTES");
        std::env::remove_var("SPINE_ZONE");
    }
}
