//! Integration tests for protocol frame bounds, state machine invariants,
//! checksum verification, and directory traversal defense.

use spine_net::{handle_server_session, DEFAULT_SECRET_KEY};
use spine_protocol::{
    frame::{Frame, FrameFlags, FrameHeader, FrameType, ProtocolError, MAX_PAYLOAD_SIZE},
    messages::{ErrorMessage, HelloMessage, ProtocolMessage, RepoOpen},
};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn test_frame_bounds_and_checksum_corruption() {
    // 1. Frame exceeding MAX_PAYLOAD_SIZE is rejected
    let huge_header = FrameHeader::new(
        FrameType::Hello,
        FrameFlags::empty(),
        (MAX_PAYLOAD_SIZE + 1) as u32,
        [0u8; 4],
    );
    let bytes = huge_header.serialize();
    let err = FrameHeader::deserialize(&bytes).unwrap_err();
    match err {
        ProtocolError::PayloadTooLarge(len, max) => {
            assert_eq!(len, MAX_PAYLOAD_SIZE + 1);
            assert_eq!(max, MAX_PAYLOAD_SIZE);
        }
        other => panic!("Expected PayloadTooLarge, got {:?}", other),
    }

    // 2. Frame with corrupted payload on the wire is rejected via checksum
    let hello = HelloMessage {
        client_version: "0.1.0".to_string(),
        capabilities: vec![],
        agent: "test".to_string(),
    };
    let frame = hello.to_frame().unwrap();
    let mut wire_bytes = frame.serialize_to_vec();
    let last = wire_bytes.len() - 1;
    wire_bytes[last] ^= 0xFF; // Corrupt payload byte on wire

    let err = Frame::read_from_sync(&wire_bytes[..]).unwrap_err();
    match err {
        ProtocolError::ChecksumMismatch(_, _) => {}
        other => panic!("Expected ChecksumMismatch, got {:?}", other),
    }
}

#[tokio::test]
async fn test_state_machine_violation_rejection() {
    let server_root = tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let root_path: PathBuf = server_root.path().to_path_buf();
    let secret = *DEFAULT_SECRET_KEY;

    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let r_root = root_path.clone();
            tokio::spawn(async move {
                let _ = handle_server_session(socket, r_root, secret).await;
            });
        }
    });

    // Client connects and sends RepoOpen immediately without HELLO or AUTH
    let mut stream = TcpStream::connect(("127.0.0.1", server_port))
        .await
        .unwrap();

    let illegal_repo_open = RepoOpen {
        path: "test/repo".to_string(),
    };
    illegal_repo_open
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();

    // Server should terminate session due to state machine violation
    let response = Frame::read_from_async(&mut stream).await;
    // Expected to receive EOF or connection reset
    assert!(response.is_err(), "Server must reject out-of-order frame");
}

#[tokio::test]
async fn test_directory_traversal_defense() {
    let server_root = tempdir().unwrap();
    let outside_dir = tempdir().unwrap();
    // Create a target repo outside server root
    let outside_repo = outside_dir.path().join("forbidden-repo");
    fs::create_dir_all(outside_repo.join(".spine").join("objects")).unwrap();
    fs::create_dir_all(outside_repo.join(".spine").join("refs").join("heads")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let server_port = listener.local_addr().unwrap().port();
    let root_path: PathBuf = server_root.path().to_path_buf();
    let secret = *DEFAULT_SECRET_KEY;

    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let r_root = root_path.clone();
            tokio::spawn(async move {
                let _ = handle_server_session(socket, r_root, secret).await;
            });
        }
    });

    let mut stream = TcpStream::connect(("127.0.0.1", server_port))
        .await
        .unwrap();

    // 1. Send Hello
    let hello = HelloMessage {
        client_version: "0.1.0".to_string(),
        capabilities: vec![],
        agent: "audit-client".to_string(),
    };
    hello
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();
    let _ack = Frame::read_from_async(&mut stream).await.unwrap();

    // 2. Auth challenge & response
    let challenge_frame = Frame::read_from_async(&mut stream).await.unwrap();
    let challenge = spine_protocol::messages::AuthChallenge::from_frame(&challenge_frame).unwrap();

    let sig = spine_net::compute_auth_signature(
        DEFAULT_SECRET_KEY,
        "auditor",
        &challenge.nonce,
        challenge.timestamp_ms,
    );
    let auth_resp = spine_protocol::messages::AuthResponse {
        user_id: "auditor".to_string(),
        signature: sig.to_vec(),
        public_key: None,
        key_id: None,
        scope: None,
    };
    auth_resp
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();
    let _auth_res = Frame::read_from_async(&mut stream).await.unwrap();

    // 3. Attempt directory traversal: "../../forbidden-repo"
    let attack_repo_open = RepoOpen {
        path: "../../forbidden-repo".to_string(),
    };
    attack_repo_open
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();

    let response_frame = Frame::read_from_async(&mut stream).await.unwrap();
    assert_eq!(response_frame.header.frame_type, FrameType::Error);
    let err_msg = ErrorMessage::from_frame(&response_frame).unwrap();
    assert_eq!(err_msg.code, 404); // Returns 404 Not Found to prevent leaking presence
}
