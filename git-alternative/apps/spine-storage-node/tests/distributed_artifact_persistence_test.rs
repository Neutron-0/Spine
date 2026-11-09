use spine_storage_node::backend::{FileSystemStorageBackend, StorageBackend};
use tempfile::tempdir;

#[test]
fn test_storage_node_artifact_persistence_and_restart() {
    let tmp = tempdir().unwrap();
    let node1_dir = tmp.path().join("node1");
    let node2_dir = tmp.path().join("node2");
    let capacity = 100 * 1024 * 1024; // 100 MB

    let chunk_bytes = b"SPINE_DISTRIBUTED_CHUNK_PAYLOAD_TEST_BYTES_123456789";
    let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();

    let manifest_bytes = br#"{"version":1,"artifact_id":"test_art","chunks":[]}"#;
    let manifest_id = blake3::hash(manifest_bytes).to_hex().to_string();

    // 1. Upload to Node 1
    {
        let node1 = FileSystemStorageBackend::new(&node1_dir, capacity).unwrap();
        node1.store_chunk(&chunk_hash, chunk_bytes).unwrap();
        node1.store_manifest(&manifest_id, manifest_bytes).unwrap();

        assert_eq!(node1.get_chunk(&chunk_hash).unwrap().unwrap(), chunk_bytes);
        assert_eq!(
            node1.get_manifest(&manifest_id).unwrap().unwrap(),
            manifest_bytes
        );
    } // node1 dropped (simulating process exit/shutdown)

    // 2. Restart Node 1: Verify data persists across process restart
    {
        let node1_restarted = FileSystemStorageBackend::new(&node1_dir, capacity).unwrap();
        let loaded_chunk = node1_restarted.get_chunk(&chunk_hash).unwrap().unwrap();
        let loaded_manifest = node1_restarted.get_manifest(&manifest_id).unwrap().unwrap();

        assert_eq!(loaded_chunk, chunk_bytes);
        assert_eq!(loaded_manifest, manifest_bytes);

        // Integrity verification
        assert_eq!(blake3::hash(&loaded_chunk).to_hex().to_string(), chunk_hash);
        assert_eq!(
            blake3::hash(&loaded_manifest).to_hex().to_string(),
            manifest_id
        );
    }

    // 3. Replicate from Node 1 to Node 2 (Replica Creation)
    {
        let node1 = FileSystemStorageBackend::new(&node1_dir, capacity).unwrap();
        let node2 = FileSystemStorageBackend::new(&node2_dir, capacity).unwrap();

        // Node 2 initially empty
        assert!(node2.get_chunk(&chunk_hash).unwrap().is_none());
        assert!(node2.get_manifest(&manifest_id).unwrap().is_none());

        // Stream replication
        let chunk_data = node1.get_chunk(&chunk_hash).unwrap().unwrap();
        node2.store_chunk(&chunk_hash, &chunk_data).unwrap();

        let manifest_data = node1.get_manifest(&manifest_id).unwrap().unwrap();
        node2.store_manifest(&manifest_id, &manifest_data).unwrap();

        // Node 2 has identical data
        assert_eq!(node2.get_chunk(&chunk_hash).unwrap().unwrap(), chunk_bytes);
        assert_eq!(
            node2.get_manifest(&manifest_id).unwrap().unwrap(),
            manifest_bytes
        );
    }

    // 4. Restart Node 2 and verify replica persistence
    {
        let node2_restarted = FileSystemStorageBackend::new(&node2_dir, capacity).unwrap();
        assert_eq!(
            node2_restarted.get_chunk(&chunk_hash).unwrap().unwrap(),
            chunk_bytes
        );
        assert_eq!(
            node2_restarted.get_manifest(&manifest_id).unwrap().unwrap(),
            manifest_bytes
        );
    }
}

#[test]
fn test_fencing_generation_persistence_and_restore_finalization() {
    let tmp = tempdir().unwrap();
    let node_dir = tmp.path().join("node_fencing");
    let capacity = 100 * 1024 * 1024;

    let snapshot_id = "snap_42";
    let fencing_gen = 55u64;
    let original_payload = b"ORIGINAL_STORAGE_PAYLOAD_GEN_1";
    let original_hash = blake3::hash(original_payload).to_hex().to_string();

    {
        let backend = FileSystemStorageBackend::new(&node_dir, capacity).unwrap();
        backend
            .store_chunk(&original_hash, original_payload)
            .unwrap();
        assert_eq!(backend.get_maintenance_status().1, 1);

        // Enter maintenance mode before volume restore operations
        backend.enter_maintenance(fencing_gen).unwrap();

        // Stage and commit a volume restore to bump fencing generation
        let (snap_manifest, snap_tar) = backend
            .capture_volume_snapshot(snapshot_id, fencing_gen)
            .unwrap();
        assert_eq!(snap_manifest.fencing_generation, fencing_gen);

        backend
            .stage_volume_restore(snapshot_id, fencing_gen, &snap_tar)
            .unwrap();
        backend
            .commit_volume_restore(snapshot_id, fencing_gen)
            .unwrap();

        backend.exit_maintenance(fencing_gen).unwrap();

        assert_eq!(backend.get_maintenance_status().1, fencing_gen);

        // Verify .old_restore_<id> is retained
        let old_restore_dir = node_dir.join(format!(".old_restore_{}", snapshot_id));
        assert!(
            old_restore_dir.exists(),
            "old_restore must be retained after commit"
        );

        // Finalize volume restore
        backend.finalize_volume_restore(snapshot_id).unwrap();
        assert!(
            !old_restore_dir.exists(),
            "old_restore must be cleaned up after finalize"
        );
    }

    // Restart backend on same directory: verify fencing generation reloaded from disk
    {
        let restarted = FileSystemStorageBackend::new(&node_dir, capacity).unwrap();
        assert_eq!(
            restarted.get_maintenance_status().1,
            fencing_gen,
            "Fencing generation must survive process restart"
        );
    }
}
