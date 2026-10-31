//! Spine Network Transport, Wire Protocol Client, and Server Session Management.

pub mod auth;
pub mod client;
pub mod negotiation;
pub mod pack_transfer;
pub mod server;
pub mod url;

pub use auth::{compute_auth_signature, verify_auth_signature, DEFAULT_SECRET_KEY};
pub use client::{ClientError, SpineClient};
pub use negotiation::compute_missing_objects;
pub use pack_transfer::{create_pack_file, receive_and_ingest_pack, send_pack_file, TransferError};
pub use server::{handle_server_session, scan_repository_refs, ServerError};
pub use url::{SpineUrl, UrlError, DEFAULT_SPINE_PORT};

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::{Commit, FileMode, HashAlgorithm, ObjectType, TreeEntry};
    use spine_format::{serialize_commit, serialize_tree};
    use spine_object_store::{CompositeObjectStore, ObjectStore};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn test_negotiation_delta_computation() {
        let temp = tempdir().unwrap();
        let store = CompositeObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        // 1. Create base commit C1
        let blob1 = store.write_object(ObjectType::Blob, b"file1 v1").unwrap();
        let tree1_entries = vec![TreeEntry {
            name: "file1.txt".to_string(),
            mode: FileMode::Regular,
            object_id: blob1,
        }];
        let tree1_bytes = serialize_tree(&tree1_entries);
        let tree1 = store.write_object(ObjectType::Tree, &tree1_bytes).unwrap();

        let c1_obj = Commit::new_simple(
            tree1,
            vec![],
            "Author".to_string(),
            "author@example.com".to_string(),
            100,
            "Initial commit".to_string(),
        );
        let c1_bytes = serialize_commit(&c1_obj);
        let c1 = store.write_object(ObjectType::Commit, &c1_bytes).unwrap();

        // 2. Create child commit C2 with new blob
        let blob2 = store.write_object(ObjectType::Blob, b"file2 v1").unwrap();
        let tree2_entries = vec![
            TreeEntry {
                name: "file1.txt".to_string(),
                mode: FileMode::Regular,
                object_id: blob1,
            },
            TreeEntry {
                name: "file2.txt".to_string(),
                mode: FileMode::Regular,
                object_id: blob2,
            },
        ];
        let tree2_bytes = serialize_tree(&tree2_entries);
        let tree2 = store.write_object(ObjectType::Tree, &tree2_bytes).unwrap();

        let c2_obj = Commit::new_simple(
            tree2,
            vec![c1],
            "Author".to_string(),
            "author@example.com".to_string(),
            200,
            "Second commit".to_string(),
        );
        let c2_bytes = serialize_commit(&c2_obj);
        let c2 = store.write_object(ObjectType::Commit, &c2_bytes).unwrap();

        // Negotiate: client wants C2, but already has C1
        let delta = compute_missing_objects(&store, &[c2], &[c1]).unwrap();

        // Delta MUST contain C2, Tree2, and Blob2, but MUST NOT contain C1, Tree1, or Blob1!
        assert!(delta.contains(&c2));
        assert!(delta.contains(&tree2));
        assert!(delta.contains(&blob2));

        assert!(!delta.contains(&c1), "C1 should not be in delta");
        assert!(!delta.contains(&blob1), "Blob1 should not be in delta");
    }

    #[tokio::test]
    async fn test_client_server_loopback_handshake_and_ref_advertisement() {
        let temp_server_root = tempdir().unwrap();
        let repo_path = temp_server_root.path().join("test-repo");
        let spine_dir = repo_path.join(".spine");
        fs::create_dir_all(&spine_dir).unwrap();

        // Initialize bare minimum repo structures
        let store = CompositeObjectStore::new(&spine_dir, HashAlgorithm::Blake3, 3).unwrap();
        let b = store
            .write_object(ObjectType::Blob, b"test content")
            .unwrap();
        let c = Commit::new_simple(
            b,
            vec![],
            "Author".to_string(),
            "author@example.com".to_string(),
            1,
            "Initial commit".to_string(),
        );
        let c_bytes = serialize_commit(&c);
        let commit_oid = store.write_object(ObjectType::Commit, &c_bytes).unwrap();

        let heads_dir = spine_dir.join("refs").join("heads");
        fs::create_dir_all(&heads_dir).unwrap();
        fs::write(heads_dir.join("main"), format!("{}\n", commit_oid.to_hex())).unwrap();

        // Start listener on ephemeral port
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let root = temp_server_root.path().to_path_buf();
        let secret = *DEFAULT_SECRET_KEY;

        tokio::spawn(async move {
            if let Ok((socket, _)) = listener.accept().await {
                let _ = handle_server_session(socket, root, secret).await;
            }
        });

        let url = format!("spine://alice@127.0.0.1:{}/test-repo", port);
        let client = SpineClient::connect(&url, None, Some(&secret))
            .await
            .unwrap();

        let refs = client.advertised_refs();
        assert_eq!(refs.get("refs/heads/main"), Some(&commit_oid));
    }
}
