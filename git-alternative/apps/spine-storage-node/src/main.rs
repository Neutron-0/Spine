pub mod backend;
pub mod server;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser, Debug)]
#[command(
    name = "spine-storage-node",
    about = "Spine Distributed Storage Node Daemon"
)]
struct Args {
    #[arg(short, long, default_value = "node-1")]
    node_id: String,

    #[arg(short, long, default_value = "127.0.0.1:9001")]
    listen: String,

    #[arg(long, default_value = ".spine-node-data")]
    storage_root: PathBuf,

    #[arg(short, long)]
    coordinator: Option<String>,

    #[arg(long, default_value_t = 53687091200)] // 50 GB
    capacity: u64,

    #[arg(long, default_value = "local")]
    region: String,

    #[arg(long, default_value = "zone-1")]
    zone: String,

    #[arg(long, default_value = "localhost")]
    host: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    println!("===========================================================");
    println!("      SPINE DISTRIBUTED STORAGE NODE DAEMON               ");
    println!("===========================================================");
    println!("Node ID:      {}", args.node_id);
    println!("Listen Addr:  {}", args.listen);
    println!("Storage Root: {}", args.storage_root.display());
    println!("Capacity:     {} bytes", args.capacity);
    println!("Domain:       {}/{}/{}", args.region, args.zone, args.host);
    if let Some(ref coord) = args.coordinator {
        println!("Coordinator:  {}", coord);
    }
    println!("===========================================================");

    let backend = Arc::new(backend::FileSystemStorageBackend::new(
        &args.storage_root,
        args.capacity,
    )?);

    let server =
        server::StorageNodeServer::new(args.node_id, args.listen, args.coordinator, backend);

    server.run().await?;
    Ok(())
}
