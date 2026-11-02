//! Integration tests for server-side transitive object closure validation.

use spine_core::{Commit, FileMode, HashAlgorithm, ObjectId, ObjectType, TreeEntry};
use spine_format::{serialize_commit, serialize_tree};
use spine_net::{create_pack_file, handle_server_session, send_pack_file, DEFAULT_SECRET_KEY};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use spine_protocol::{
    frame::Frame,
    messages::{ProtocolMessage, PushBegin, RefUpdateProposal, RefUpdateReq, RefUpdateRes},
};
use std::fs;
use std::path::PathBuf;
use tempfile::tempdir;
use tokio::net::{TcpListener, TcpStream};

#[tokio::test]
async fn test_push_object_closure_rejection_on_missing_blob() {
    let server_root = tempdir().unwrap();
    let repo_dir = server_root.path().join("closure-repo");
    fs::create_dir_all(repo_dir.join(".spine").join("objects")).unwrap();
    fs::create_dir_all(repo_dir.join(".spine").join("refs").join("heads")).unwrap();

    let server_store =
        CompositeObjectStore::new(repo_dir.join(".spine"), HashAlgorithm::Blake3, 3).unwrap();

    // 1. Create a legitimate base commit on server
    let base_blob = server_store
        .write_object(ObjectType::Blob, b"base content")
        .unwrap();
    let base_tree = server_store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[TreeEntry {
                name: "base.txt".to_string(),
                mode: FileMode::Regular,
                object_id: base_blob,
            }]),
        )
        .unwrap();
    let base_commit = server_store
        .write_object(
            ObjectType::Commit,
            &serialize_commit(&Commit::new_simple(
                base_tree,
                vec![],
                "Author".to_string(),
                "author@spine.io".to_string(),
                100,
                "Base commit".to_string(),
            )),
        )
        .unwrap();

    fs::write(
        repo_dir
            .join(".spine")
            .join("refs")
            .join("heads")
            .join("main"),
        format!("{}\n", base_commit.to_hex()),
    )
    .unwrap();

    // 2. Start daemon
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

    let _repo_url = format!("spine://127.0.0.1:{}/closure-repo", server_port);

    // 3. Client creates an invalid commit referencing a phantom blob
    let client_temp = tempdir().unwrap();
    let client_store =
        CompositeObjectStore::new(client_temp.path(), HashAlgorithm::Blake3, 3).unwrap();

    let phantom_blob_oid = ObjectId::from_bytes([0xEE; 32]);
    let bad_tree = client_store
        .write_object(
            ObjectType::Tree,
            &serialize_tree(&[TreeEntry {
                name: "ghost.txt".to_string(),
                mode: FileMode::Regular,
                object_id: phantom_blob_oid,
            }]),
        )
        .unwrap();

    let bad_commit = client_store
        .write_object(
            ObjectType::Commit,
            &serialize_commit(&Commit::new_simple(
                bad_tree,
                vec![base_commit],
                "Attacker".to_string(),
                "attacker@spine.io".to_string(),
                200,
                "Commit pointing to missing blob".to_string(),
            )),
        )
        .unwrap();

    // 4. Manually create pack containing ONLY bad_commit and bad_tree (omitting the phantom blob)
    let tmp_pack = tempdir().unwrap().path().join("incomplete.spk");
    let (checksum, count) =
        create_pack_file(&client_store, &[bad_commit, bad_tree], &tmp_pack).unwrap();

    // 5. Connect and stream incomplete pack to server
    let mut stream = TcpStream::connect(("127.0.0.1", server_port))
        .await
        .unwrap();

    // Handshake
    let hello = spine_protocol::messages::HelloMessage {
        client_version: "0.1.0".to_string(),
        capabilities: vec![],
        agent: "test-client".to_string(),
    };
    hello
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();
    let _ack = Frame::read_from_async(&mut stream).await.unwrap();

    // Auth
    let ch_frame = Frame::read_from_async(&mut stream).await.unwrap();
    let challenge = spine_protocol::messages::AuthChallenge::from_frame(&ch_frame).unwrap();
    let sig = spine_net::compute_auth_signature(
        DEFAULT_SECRET_KEY,
        "admin",
        &challenge.nonce,
        challenge.timestamp_ms,
    );
    let auth_resp = spine_protocol::messages::AuthResponse {
        user_id: "admin".to_string(),
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

    // RepoOpen
    let repo_open = spine_protocol::messages::RepoOpen {
        path: "closure-repo".to_string(),
    };
    repo_open
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();
    let _repo_info = Frame::read_from_async(&mut stream).await.unwrap();
    let _ref_adv = Frame::read_from_async(&mut stream).await.unwrap();

    // PushBegin
    let proposal = RefUpdateProposal {
        ref_name: "refs/heads/main".to_string(),
        expected_old_oid: Some(base_commit),
        new_oid: bad_commit,
        force: false,
        lease_oid: None,
    };
    let push_begin = PushBegin {
        ref_updates: vec![proposal.clone()],
        expected_pack_size: None,
    };
    push_begin
        .to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();

    // Send incomplete pack
    send_pack_file(&tmp_pack, checksum, count, &mut stream)
        .await
        .unwrap();

    // Send RefUpdateReq
    let req = RefUpdateReq {
        updates: vec![proposal],
    };
    req.to_frame()
        .unwrap()
        .write_to_async(&mut stream)
        .await
        .unwrap();

    // Read RefUpdateRes
    let res_frame = Frame::read_from_async(&mut stream).await.unwrap();
    let res = RefUpdateRes::from_frame(&res_frame).unwrap();

    // The push must be rejected by server closure validation!
    assert!(
        !res.success,
        "Push with missing blob must fail closure validation"
    );
    let result = &res.results[0];
    assert_eq!(result.status, "missing_object");
    assert!(result
        .message
        .as_ref()
        .unwrap()
        .contains("Object closure validation failed"));

    // Verify remote ref was NOT modified
    let current_ref = fs::read_to_string(
        repo_dir
            .join(".spine")
            .join("refs")
            .join("heads")
            .join("main"),
    )
    .unwrap();
    assert_eq!(current_ref.trim(), base_commit.to_hex());
}
