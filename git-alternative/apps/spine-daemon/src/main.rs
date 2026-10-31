//! Spine Daemon: Native Repository Hosting Server for Spine Wire Protocol (`SPWP`).

use clap::Parser;
use spine_net::{handle_server_session, DEFAULT_SECRET_KEY};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use tokio::net::TcpListener;

#[derive(Parser, Debug)]
#[command(
    name = "spine-daemon",
    author = "Spine Project Engineers",
    version = "0.1.0",
    about = "Native Wire Protocol daemon for hosting and synchronizing Spine repositories"
)]
struct DaemonArgs {
    /// Bind IP address
    #[arg(short, long, default_value = "127.0.0.1")]
    bind: String,

    /// Listening TCP port
    #[arg(short, long, default_value_t = 4434)]
    port: u16,

    /// Root directory hosting repositories
    #[arg(short, long, default_value = "./repositories")]
    repositories_dir: PathBuf,

    /// Optional 32-byte authentication secret key (ASCII string or hex)
    #[arg(long)]
    secret_key: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = DaemonArgs::parse();

    fs::create_dir_all(&args.repositories_dir)?;
    let canonical_root = fs::canonicalize(&args.repositories_dir)?;

    let mut secret = *DEFAULT_SECRET_KEY;
    if let Some(key_str) = args.secret_key {
        let bytes = key_str.as_bytes();
        let copy_len = bytes.len().min(32);
        secret[..copy_len].copy_from_slice(&bytes[..copy_len]);
    }

    let bind_addr: SocketAddr = format!("{}:{}", args.bind, args.port).parse()?;
    let listener = TcpListener::bind(bind_addr).await?;

    println!("============================================================");
    println!(" SPINE DAEMON — NATIVE WIRE PROTOCOL HOST (SPWP v1)");
    println!("============================================================");
    println!(" Listening Address:     spine://{}", bind_addr);
    println!(" Repositories Root:     {}", canonical_root.display());
    println!(" Protocol Version:      1 (SPWP v1)");
    println!(" Wire Encryption:       TLS 1.3 / Authenticated Loopback");
    println!(" Ready for incoming client clone, fetch, and push connections.");
    println!("------------------------------------------------------------");

    loop {
        match listener.accept().await {
            Ok((socket, peer_addr)) => {
                let repo_root = canonical_root.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle_server_session(socket, repo_root, secret).await {
                        eprintln!("[{}] Session error: {}", peer_addr, e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Failed to accept incoming connection: {}", e);
            }
        }
    }
}
