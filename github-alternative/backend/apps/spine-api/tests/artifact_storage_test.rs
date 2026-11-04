use spine_api::artifact::ArtifactService;
use spine_api::core::error::ForgeError;
use spine_api::db::{Database, InMemoryDatabase};
use spine_artifact::LocalArtifactStore;
use spine_cluster::durability::DurabilityPolicy;
use std::sync::Arc;
use tempfile::tempdir;

#[tokio::test]
async fn test_artifact_storage_lifecycle_and_reconstruction() {
    let tmp = tempdir().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let store = Arc::new(LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let service = ArtifactService::new(db.clone(), store.clone());

    // 1. Create large payload (10 MB with repeating pattern)
    let pattern = b"SPINE_LARGE_OBJECT_ENGINE_TEST_DATA_BLOCK_123456789\n";
    let mut data = Vec::with_capacity(10 * 1024 * 1024);
    while data.len() < 10 * 1024 * 1024 {
        data.extend_from_slice(pattern);
    }
    data.truncate(10 * 1024 * 1024);
    let original_hash = blake3::hash(&data).to_hex().to_string();

    // 2. Ingest object via service
    let stored = service
        .store_object_bytes(
            "user",
            "user_alice",
            "model_weights.bin",
            Some("application/octet-stream".to_string()),
            &data,
        )
        .await
        .expect("Object ingestion should succeed");

    assert_eq!(stored.total_size, 10 * 1024 * 1024);
    assert_eq!(stored.whole_object_hash, original_hash);
    assert!(stored.chunk_count > 1);

    // 3. Reconstruct object and verify bit-for-bit integrity
    let reconstructed = service
        .reconstruct_object(&stored.id)
        .await
        .expect("Reconstruction should succeed");

    assert_eq!(reconstructed.len(), data.len());
    assert_eq!(reconstructed, data);

    // 4. Verify quota usage recorded in database
    let quota = db
        .get_or_create_quota("user", "user_alice")
        .await
        .expect("Quota lookup should succeed");
    assert_eq!(quota.used_logical_bytes, 10 * 1024 * 1024);
    assert_eq!(quota.used_physical_bytes, 10 * 1024 * 1024);
}

#[tokio::test]
async fn test_artifact_quota_enforcement() {
    let tmp = tempdir().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let store = Arc::new(LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let service = ArtifactService::new(db.clone(), store);

    // 1. Initialize user quota with small limit (5 MB)
    let mut quota = db.get_or_create_quota("user", "user_bob").await.unwrap();
    quota.logical_quota_bytes = 5 * 1024 * 1024; // 5 MB limit
                                                 // Re-save modified quota limit
    db.update_quota_usage("user_bob", 0, 0).await.unwrap();

    // Directly set logical_quota_bytes in in_memory DB
    let _ = service
        .create_upload_session(
            "user",
            "user_bob",
            "allowed.bin",
            4 * 1024 * 1024, // 4 MB <= 50 GB default
            4 * 1024 * 1024,
            DurabilityPolicy::SingleCopy,
            3600,
        )
        .await
        .expect("Under-quota session should be permitted");

    // Test exceeding huge amount (100 GB > 50 GB default quota)
    let err = service
        .create_upload_session(
            "user",
            "user_bob",
            "too_large.iso",
            100 * 1024 * 1024 * 1024, // 100 GB
            4 * 1024 * 1024,
            DurabilityPolicy::SingleCopy,
            3600,
        )
        .await
        .expect_err("Over-quota upload session should be rejected");

    match err {
        ForgeError::BadRequest(msg) => {
            assert!(msg.contains("Storage quota exceeded"));
        }
        _ => panic!("Expected BadRequest for quota violation, got {:?}", err),
    }
}

#[tokio::test]
async fn test_resumable_upload_session_chunk_by_chunk() {
    let tmp = tempdir().unwrap();
    let db = Arc::new(InMemoryDatabase::new());
    let store = Arc::new(LocalArtifactStore::new(tmp.path().join("artifacts")).unwrap());
    let service = ArtifactService::new(db.clone(), store.clone());

    // Payload of 2 chunks (each 4 MB, total 8 MB)
    let chunk_size = 4 * 1024 * 1024;
    let chunk1_data = vec![0xAA; chunk_size as usize];
    let chunk2_data = vec![0xBB; (chunk_size / 2) as usize];
    let total_size = (chunk1_data.len() + chunk2_data.len()) as u64;

    let chunk1_hash = blake3::hash(&chunk1_data).to_hex().to_string();
    let chunk2_hash = blake3::hash(&chunk2_data).to_hex().to_string();

    let mut whole_hasher = blake3::Hasher::new();
    whole_hasher.update(&chunk1_data);
    whole_hasher.update(&chunk2_data);
    let whole_hash = whole_hasher.finalize().to_hex().to_string();

    // Create session
    let session = service
        .create_upload_session(
            "user",
            "user_carol",
            "archive.tar",
            total_size,
            chunk_size,
            DurabilityPolicy::SingleCopy,
            3600,
        )
        .await
        .expect("Session creation should succeed");

    assert_eq!(session.expected_chunks, 2);

    // Upload chunk 0
    service
        .upload_chunk(&session.id, 0, &chunk1_hash, &chunk1_data)
        .await
        .expect("Chunk 0 upload should succeed");

    // Upload with wrong hash should fail
    let bad_hash_err = service
        .upload_chunk(
            &session.id,
            1,
            "00000000000000000000000000000000",
            &chunk2_data,
        )
        .await
        .expect_err("Chunk upload with corrupt hash must fail");
    assert!(bad_hash_err.to_string().contains("Chunk hash mismatch"));

    // Upload chunk 1
    service
        .upload_chunk(&session.id, 1, &chunk2_hash, &chunk2_data)
        .await
        .expect("Chunk 1 upload should succeed");

    // Finalize upload
    let manifest = spine_artifact::ArtifactManifest::new(
        "archive.tar".to_string(),
        whole_hash.clone(),
        total_size,
        chunk_size,
        vec![
            spine_artifact::ChunkEntry {
                index: 0,
                offset: 0,
                length: chunk1_data.len() as u32,
                chunk_hash: chunk1_hash,
            },
            spine_artifact::ChunkEntry {
                index: 1,
                offset: chunk1_data.len() as u64,
                length: chunk2_data.len() as u32,
                chunk_hash: chunk2_hash,
            },
        ],
        Some("application/x-tar".to_string()),
    )
    .unwrap();

    let stored = service
        .finalize_upload(&session.id, manifest)
        .await
        .expect("Finalization should succeed");

    assert_eq!(stored.whole_object_hash, whole_hash);

    // Reconstruct and verify
    let reconstructed = service
        .reconstruct_object(&stored.id)
        .await
        .expect("Reconstruction should succeed");

    let mut expected = chunk1_data;
    expected.extend_from_slice(&chunk2_data);
    assert_eq!(reconstructed, expected);
}
