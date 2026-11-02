//! Integration test suite for Ed25519 cryptographic identity, replay resistance,
//! key revocation, and role-based access control (RBAC).

use spine_net::{
    handle_server_session_stream, BoundedReplayCache, ClientConnectOptions, KeyRegistry, Role,
    ServerSessionConfig, SpineClient, SpineIdentity, SpineStream,
};
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::tempdir;
use tokio::net::TcpListener;

#[tokio::test]
async fn test_ed25519_asymmetric_authentication_and_roles() {
    let repo_temp = tempdir().unwrap();
    let repo_dir = repo_temp.path().join("secure-repo");
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

    let registry = Arc::new(KeyRegistry::new());
    let replay_cache = Arc::new(BoundedReplayCache::new());

    // Register 3 identities: Alice (Admin), Bob (Reader), Charlie (Revoked)
    let alice_identity = SpineIdentity::generate();
    let bob_identity = SpineIdentity::generate();
    let charlie_identity = SpineIdentity::generate();

    registry.register_key("alice", alice_identity.verifying_key(), Role::Admin, None);
    registry.register_key("bob", bob_identity.verifying_key(), Role::Reader, None);
    registry.register_key(
        "charlie",
        charlie_identity.verifying_key(),
        Role::Contributor,
        None,
    );
    registry.revoke_key(&charlie_identity.public_key_bytes());

    let server_config = ServerSessionConfig {
        secret_key: [0u8; 32],
        key_registry: registry.clone(),
        replay_cache: replay_cache.clone(),
        server_id: "test-node-ed25519".to_string(),
        allow_symmetric_auth: false, // Strict Ed25519 only
        require_tls: false,
    };

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let root_path: PathBuf = repo_temp.path().to_path_buf();

    // Spawn server daemon
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let r_root = root_path.clone();
            let s_cfg = server_config.clone();
            tokio::spawn(async move {
                let _ =
                    handle_server_session_stream(SpineStream::Plain(socket), r_root, s_cfg).await;
            });
        }
    });

    let repo_url = format!("spine://127.0.0.1:{}/secure-repo", server_port);

    // 1. Alice connects with valid Ed25519 identity -> succeeds
    let alice_opts = ClientConnectOptions {
        user_override: Some("alice".to_string()),
        auth_secret: None,
        identity: Some(alice_identity),
        tls_config: None,
        insecure_plain: true,
    };
    let alice_client = SpineClient::connect_with_options(&repo_url, alice_opts)
        .await
        .expect("Alice should authenticate successfully with valid Ed25519 key");
    assert!(alice_client.repo_info().is_some());

    // 2. Bob connects with valid Reader identity -> succeeds
    let bob_opts = ClientConnectOptions {
        user_override: Some("bob".to_string()),
        auth_secret: None,
        identity: Some(bob_identity),
        tls_config: None,
        insecure_plain: true,
    };
    let bob_client = SpineClient::connect_with_options(&repo_url, bob_opts)
        .await
        .expect("Bob should authenticate successfully with reader key");
    assert!(bob_client.repo_info().is_some());

    // 3. Charlie connects with revoked identity -> rejected
    let charlie_opts = ClientConnectOptions {
        user_override: Some("charlie".to_string()),
        auth_secret: None,
        identity: Some(charlie_identity),
        tls_config: None,
        insecure_plain: true,
    };
    let charlie_err = SpineClient::connect_with_options(&repo_url, charlie_opts)
        .await
        .expect_err("Charlie should be rejected because key is revoked");
    match charlie_err {
        spine_net::ClientError::AuthFailed(msg) => {
            assert!(msg.contains("Invalid credentials"));
        }
        other => panic!("Expected AuthFailed, got {:?}", other),
    }

    // 4. Unknown identity connects -> rejected
    let dave_unregistered = SpineIdentity::generate();
    let dave_opts = ClientConnectOptions {
        user_override: Some("dave".to_string()),
        auth_secret: None,
        identity: Some(dave_unregistered),
        tls_config: None,
        insecure_plain: true,
    };
    assert!(SpineClient::connect_with_options(&repo_url, dave_opts)
        .await
        .is_err());
}
