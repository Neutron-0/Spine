use spine_artifact::{
    ArtifactGcCollector, ArtifactStore, LfsPointer, LfsTrackingRules, LocalArtifactStore,
    ObjectReconstructor, StreamChunker, DEFAULT_CHUNK_SIZE,
};
use spine_cluster::durability::DurabilityPolicy;
use std::fs;
use std::io::Cursor;
use tempfile::tempdir;

#[test]
fn test_lfs_full_remote_lifecycle_and_durability_and_gc() {
    let tmp = tempdir().unwrap();
    let root = tmp.path();

    let remote_dir = root.join("remote_repo.spine");
    let local_dir = root.join("local_worktree");
    let clone_dir = root.join("cloned_worktree");

    fs::create_dir_all(&remote_dir).unwrap();
    fs::create_dir_all(&local_dir).unwrap();
    fs::create_dir_all(&clone_dir).unwrap();

    let local_store = LocalArtifactStore::new(local_dir.join(".spine").join("artifacts")).unwrap();
    let remote_store = LocalArtifactStore::new(remote_dir.join("artifacts")).unwrap();
    let clone_store = LocalArtifactStore::new(clone_dir.join(".spine").join("artifacts")).unwrap();

    // 1. Initialize local repository & track pattern
    let mut rules = LfsTrackingRules::default();
    rules.track_pattern("*.bin");
    rules.save_to_file(local_dir.join(".spinelfs")).unwrap();

    // 2. Create a 10 MiB binary payload with distinct pseudo-random pattern
    let mut original_data = vec![0u8; 10 * 1024 * 1024];
    for (i, byte) in original_data.iter_mut().enumerate() {
        *byte = (((i * 31 + 17) ^ (i >> 16)) % 256) as u8;
    }
    let original_hash = blake3::hash(&original_data).to_hex().to_string();
    fs::write(local_dir.join("weights.bin"), &original_data).unwrap();

    // 3. Process LFS add: chunk into local artifact store and stage pointer
    let chunker = StreamChunker::new(Some(DEFAULT_CHUNK_SIZE));
    let chunk_res = chunker
        .chunk_reader(Cursor::new(&original_data), "weights.bin", None)
        .unwrap();

    assert_eq!(chunk_res.manifest.chunks.len(), 3); // 4MB + 4MB + 2MB
    assert_eq!(chunk_res.manifest.whole_object_hash, original_hash);

    for chunk in &chunk_res.chunks {
        local_store
            .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
            .unwrap();
    }
    let manifest_oid = local_store.store_manifest(&chunk_res.manifest).unwrap();

    let pointer = LfsPointer::new(
        manifest_oid.clone(),
        chunk_res.manifest.total_size,
        chunk_res.manifest.whole_object_hash.clone(),
    );
    let pointer_bytes = pointer.encode().into_bytes();
    fs::write(local_dir.join("weights.bin.ptr"), &pointer_bytes).unwrap();

    // 4. Remote LFS Push: transfer manifest and missing chunks to remote store
    let manifest = local_store.get_manifest(&manifest_oid).unwrap().unwrap();
    for chunk in &manifest.chunks {
        if !remote_store.has_chunk(&chunk.chunk_hash) {
            let bytes = local_store.get_chunk(&chunk.chunk_hash).unwrap().unwrap();
            remote_store.store_chunk(&chunk.chunk_hash, &bytes).unwrap();
        }
    }
    remote_store.store_manifest(&manifest).unwrap();

    // 5. Confirm configured durability policy on remote store
    assert!(remote_store
        .verify_artifact_durability(&manifest_oid, DurabilityPolicy::SingleCopy)
        .unwrap());

    // 6. Clone: clean directory receives pointer file and .spinelfs
    rules.save_to_file(clone_dir.join(".spinelfs")).unwrap();
    fs::write(clone_dir.join("weights.bin"), &pointer_bytes).unwrap();

    // Initially cloned worktree only has pointer, not the 10 MB payload
    let clone_pre_meta = fs::metadata(clone_dir.join("weights.bin")).unwrap();
    assert!(clone_pre_meta.len() < 500);

    // 7. Remote LFS Fetch: transfer from remote store to clone store
    let clone_manifest = remote_store.get_manifest(&manifest_oid).unwrap().unwrap();
    clone_store.store_manifest(&clone_manifest).unwrap();
    for chunk in &clone_manifest.chunks {
        let bytes = remote_store.get_chunk(&chunk.chunk_hash).unwrap().unwrap();
        clone_store.store_chunk(&chunk.chunk_hash, &bytes).unwrap();
    }

    // 8. Hydrate object in clone directory and compare byte-for-byte
    let mut hydrated_data = Vec::new();
    ObjectReconstructor::reconstruct(
        &clone_manifest,
        |idx| {
            let entry = &clone_manifest.chunks[idx as usize];
            Ok(clone_store.get_chunk(&entry.chunk_hash).unwrap().unwrap())
        },
        &mut hydrated_data,
    )
    .unwrap();

    fs::write(clone_dir.join("weights.bin"), &hydrated_data).unwrap();
    assert_eq!(hydrated_data.len(), original_data.len());
    assert_eq!(hydrated_data, original_data);
    assert_eq!(
        blake3::hash(&hydrated_data).to_hex().to_string(),
        original_hash
    );

    // 9. Subsequent change (version 2): edit first chunk only, rest shared
    let mut v2_data = original_data.clone();
    for byte in &mut v2_data[0..1024] {
        *byte ^= 0xFF; // Modify beginning of chunk 0
    }
    let v2_hash = blake3::hash(&v2_data).to_hex().to_string();
    assert_ne!(v2_hash, original_hash);

    let v2_chunk_res = chunker
        .chunk_reader(Cursor::new(&v2_data), "weights.bin", None)
        .unwrap();
    assert_eq!(v2_chunk_res.manifest.chunks.len(), 3);
    // Chunk 1 and Chunk 2 should be IDENTICAL to version 1!
    assert_ne!(
        v2_chunk_res.manifest.chunks[0].chunk_hash,
        chunk_res.manifest.chunks[0].chunk_hash
    );
    assert_eq!(
        v2_chunk_res.manifest.chunks[1].chunk_hash,
        chunk_res.manifest.chunks[1].chunk_hash
    );
    assert_eq!(
        v2_chunk_res.manifest.chunks[2].chunk_hash,
        chunk_res.manifest.chunks[2].chunk_hash
    );

    for chunk in &v2_chunk_res.chunks {
        remote_store
            .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
            .unwrap();
    }
    let v2_manifest_oid = remote_store.store_manifest(&v2_chunk_res.manifest).unwrap();

    // 10. Verify GC preserves all reachable chunks across both versions
    let active_manifests = vec![manifest_oid.clone(), v2_manifest_oid.clone()];
    let mut all_chunks = Vec::new();
    for c in &chunk_res.manifest.chunks {
        all_chunks.push(c.chunk_hash.clone());
    }
    for c in &v2_chunk_res.manifest.chunks {
        all_chunks.push(c.chunk_hash.clone());
    }
    all_chunks.sort();
    all_chunks.dedup();

    let gc_report =
        ArtifactGcCollector::collect_garbage(&remote_store, &active_manifests, &all_chunks, false)
            .unwrap();

    // All chunks in v1 and v2 must be preserved (0 candidate orphans)
    assert_eq!(gc_report.candidate_chunks, 0);
    assert_eq!(gc_report.reachable_chunks, 4); // 2 modified chunk 0s + 1 shared chunk 1 + 1 shared chunk 2
    assert_eq!(gc_report.reclaimed_bytes, 0);
    assert_eq!(gc_report.reachable_chunks, 4); // 2 modified chunk 0s + 1 shared chunk 1 + 1 shared chunk 2
    assert_eq!(gc_report.reclaimed_bytes, 0);
}
