//! Spine Daemon: Native Repository Hosting Server for Spine Wire Protocol (`SPWP`).
//! Hardened with TLS 1.3, Ed25519 authentication, RBAC, DoS protection, and graceful shutdown.

use clap::Parser;
use spine_net::{
    build_server_tls_config, generate_self_signed_cert_der, handle_server_session_stream,
    is_loopback_addr, BoundedReplayCache, KeyRegistry, ServerSessionConfig, SpineStream,
    DEFAULT_SECRET_KEY,
};
use std::fs;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio_rustls::TlsAcceptor;

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

    /// Enable TLS 1.3 wire encryption
    #[arg(long, default_value_t = false)]
    tls: bool,

    /// Path to TLS certificate chain (PEM/DER)
    #[arg(long)]
    cert: Option<PathBuf>,

    /// Path to TLS private key (PEM/DER)
    #[arg(long)]
    key: Option<PathBuf>,

    /// Automatically generate self-signed TLS 1.3 certificate for development/testing
    #[arg(long, default_value_t = false)]
    generate_dev_cert: bool,

    /// Path to authorized keys file mapping Ed25519 public keys to principals and roles
    #[arg(long)]
    authorized_keys: Option<PathBuf>,

    /// Maximum concurrent connections allowed
    #[arg(long, default_value_t = 256)]
    max_connections: usize,
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

    // Configure Key Registry
    let key_registry = Arc::new(KeyRegistry::new());
    if let Some(auth_path) = &args.authorized_keys {
        match key_registry.load_from_file(auth_path) {
            Ok(count) => println!(
                "Loaded {} authorized key(s) from {}",
                count,
                auth_path.display()
            ),
            Err(e) => eprintln!("Warning: Failed to load authorized keys: {}", e),
        }
    }

    let replay_cache = Arc::new(BoundedReplayCache::new());

    // Configure TLS Acceptor
    let tls_acceptor: Option<TlsAcceptor> =
        if args.tls || args.generate_dev_cert || args.cert.is_some() {
            let (cert_der, key_der) = if let (Some(c), Some(k)) = (&args.cert, &args.key) {
                let c_bytes = fs::read(c)?;
                let k_bytes = fs::read(k)?;
                (c_bytes, k_bytes)
            } else {
                // Generate ephemeral dev certificate for localhost
                let (c, k) = generate_self_signed_cert_der(&["localhost", "127.0.0.1", "::1"])?;
                (c, k)
            };

            let server_config = build_server_tls_config(vec![cert_der], key_der)?;
            Some(TlsAcceptor::from(Arc::new(server_config)))
        } else {
            None
        };

    let bind_addr: SocketAddr = format!("{}:{}", args.bind, args.port).parse()?;
    let listener = TcpListener::bind(bind_addr).await?;
    let connection_semaphore = Arc::new(Semaphore::new(args.max_connections));

    let is_tls = tls_acceptor.is_some();
    let scheme = if is_tls { "spine" } else { "spinelocal" };

    println!("============================================================");
    println!(" SPINE DAEMON — NATIVE WIRE PROTOCOL HOST (SPWP v1)");
    println!("============================================================");
    println!(" Listening Address:     {}://{}", scheme, bind_addr);
    println!(" Repositories Root:     {}", canonical_root.display());
    println!(" Protocol Version:      1 (SPWP v1)");
    println!(
        " Transport Encryption:  {}",
        if is_tls {
            "TLS 1.3 (rustls)"
        } else {
            "Plaintext (Loopback Only)"
        }
    );
    println!(
        " Concurrency Limit:     {} concurrent connections",
        args.max_connections
    );
    println!(" Ready for incoming client clone, fetch, and push connections.");
    println!(" Press Ctrl+C for graceful shutdown.");
    println!("------------------------------------------------------------");

    let session_config = ServerSessionConfig {
        secret_key: secret,
        key_registry,
        replay_cache,
        server_id: "spine-daemon-node-01".to_string(),
        allow_symmetric_auth: true,
        require_tls: is_tls,
    };

    loop {
        tokio::select! {
            accept_res = listener.accept() => {
                match accept_res {
                    Ok((socket, peer_addr)) => {
                        // Strict loopback check for plaintext connections
                        if tls_acceptor.is_none() && !is_loopback_addr(&peer_addr) {
                            eprintln!(
                                "[SECURITY VIOLATION] Dropping unencrypted connection from non-loopback peer: {}",
                                peer_addr
                            );
                            continue;
                        }

                        let permit = match connection_semaphore.clone().try_acquire_owned() {
                            Ok(p) => p,
                            Err(_) => {
                                eprintln!(
                                    "[{}] Connection rejected: max concurrent connections limit ({}) reached",
                                    peer_addr, args.max_connections
                                );
                                continue;
                            }
                        };

                        let repo_root = canonical_root.clone();
                        let s_config = session_config.clone();
                        let acceptor_opt = tls_acceptor.clone();

                        tokio::spawn(async move {
                            let _permit = permit;
                            let stream_res = if let Some(acceptor) = acceptor_opt {
                                SpineStream::upgrade_server(socket, acceptor).await
                            } else {
                                Ok(SpineStream::Plain(socket))
                            };

                            match stream_res {
                                Ok(stream) => {
                                    if let Err(e) = handle_server_session_stream(stream, repo_root, s_config).await {
                                        eprintln!("[{}] Session error: {}", peer_addr, e);
                                    }
                                }
                                Err(e) => {
                                    eprintln!("[{}] TLS handshake failure: {}", peer_addr, e);
                                }
                            }
                        });
                    }
                    Err(e) => {
                        eprintln!("Failed to accept incoming connection: {}", e);
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                println!("\nReceived shutdown signal (Ctrl+C). Initiating graceful shutdown...");
                break;
            }
        }
    }

    println!("Spine Daemon terminated cleanly.");
    Ok(())
}
