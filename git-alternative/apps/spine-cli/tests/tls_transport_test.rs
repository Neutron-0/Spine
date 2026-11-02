//! Integration tests for native TLS 1.3 transport, mutual certificate validation,
//! and loopback-only plaintext enforcement.

use spine_net::{
    build_client_tls_config, build_server_tls_config, generate_self_signed_cert_der,
    handle_server_session_stream, ClientConnectOptions, ServerSessionConfig, SpineClient,
    SpineStream,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

#[tokio::test]
async fn test_tls_13_handshake_and_secure_transfer() {
    let repo_temp = tempdir().unwrap();
    let repo_dir = repo_temp.path().join("tls-repo");
    fs::create_dir_all(repo_dir.join(".spine").join("objects")).unwrap();
    fs::create_dir_all(repo_dir.join(".spine").join("refs").join("heads")).unwrap();
    fs::write(
        repo_dir
            .join(".spine")
            .join("refs")
            .join("heads")
            .join("main"),
        "0000000000000000000000000000000000000000000000000000000000000000\n",
    )
    .unwrap();

    // 1. Generate dev TLS 1.3 certificates
    let (cert_der, key_der) = generate_self_signed_cert_der(&["localhost", "127.0.0.1"]).unwrap();

    let server_tls_cfg = build_server_tls_config(vec![cert_der.clone()], key_der).unwrap();
    let acceptor = TlsAcceptor::from(Arc::new(server_tls_cfg));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let root_path: PathBuf = repo_temp.path().to_path_buf();

    let session_config = ServerSessionConfig {
        secret_key: *spine_net::DEFAULT_SECRET_KEY,
        key_registry: Arc::new(spine_net::KeyRegistry::new()),
        replay_cache: Arc::new(spine_net::BoundedReplayCache::new()),
        server_id: "test-tls-node".to_string(),
        allow_symmetric_auth: true,
        require_tls: true,
    };

    // 2. Spawn server daemon with TLS upgrade
    let acceptor_clone = acceptor.clone();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let r_root = root_path.clone();
            let s_cfg = session_config.clone();
            let acc = acceptor_clone.clone();
            tokio::spawn(async move {
                if let Ok(stream) = SpineStream::upgrade_server(socket, acc).await {
                    let _ = handle_server_session_stream(stream, r_root, s_cfg).await;
                }
            });
        }
    });

    let repo_url = format!("spine://127.0.0.1:{}/tls-repo", server_port);

    // 3. Client connects with trusted root certificate -> TLS 1.3 handshake succeeds!
    let client_tls_cfg = build_client_tls_config(Some(vec![cert_der])).unwrap();
    let client_opts = ClientConnectOptions {
        user_override: Some("alice".to_string()),
        auth_secret: None,
        identity: None,
        tls_config: Some(Arc::new(client_tls_cfg)),
        insecure_plain: false,
    };

    let client = SpineClient::connect_with_options(&repo_url, client_opts)
        .await
        .expect("TLS 1.3 connection and handshake should succeed");

    assert!(client.repo_info().is_some());
    assert_eq!(client.repo_info().unwrap().default_branch, "main");

    // 4. Untrusted client connects without certificate -> TLS handshake fails
    let untrusted_cfg = build_client_tls_config(Some(vec![])).unwrap();
    let untrusted_opts = ClientConnectOptions {
        user_override: Some("eve".to_string()),
        auth_secret: None,
        identity: None,
        tls_config: Some(Arc::new(untrusted_cfg)),
        insecure_plain: false,
    };

    let untrusted_res = SpineClient::connect_with_options(&repo_url, untrusted_opts).await;
    assert!(
        untrusted_res.is_err(),
        "Untrusted root cert must fail TLS verification"
    );
}
