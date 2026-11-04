use spine_artifact::{
    ArtifactGcCollector, ArtifactStore, LfsPointer, LfsTrackingRules, LocalArtifactStore,
    ObjectReconstructor, StreamChunker,
};
use std::io::Cursor;
use tempfile::tempdir;

#[test]
fn test_lfs_pointer_parsing_and_encoding() {
    let manifest_oid = "man_9f83c18b76e2aca2c20a9a4b";
    let size_bytes = 104_857_600u64; // 100 MB
    let object_hash = "9f83c18b76e2aca2c20a9a4b7d8b5a1936c533e46c76ab2f67ff7bb5658e385e";

    let pointer = LfsPointer::new(
        manifest_oid.to_string(),
        size_bytes,
        object_hash.to_string(),
    );

    // Test concise encoding
    let encoded = pointer.encode();
    assert!(encoded.starts_with("spine-lfs-v1 "));
    assert!(encoded.contains(manifest_oid));
    assert!(encoded.contains("104857600"));
    assert!(encoded.contains(object_hash));

    // Test detection from raw bytes
    assert!(LfsPointer::is_lfs_pointer_bytes(encoded.as_bytes()));

    // Test parsing
    let parsed = LfsPointer::parse(&encoded).expect("Pointer parse should succeed");
    assert_eq!(parsed.manifest_oid, manifest_oid);
    assert_eq!(parsed.size_bytes, size_bytes);
    assert_eq!(parsed.object_hash, object_hash);

    // Test multi-line format compatibility
    let multiline = format!(
        "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\nmanifest_oid {}\n",
        object_hash, size_bytes, manifest_oid
    );
    let parsed_multi =
        LfsPointer::parse(&multiline).expect("Multi-line pointer parse should succeed");
    assert_eq!(parsed_multi.manifest_oid, manifest_oid);
    assert_eq!(parsed_multi.size_bytes, size_bytes);
}

#[test]
fn test_lfs_tracking_rules_and_patterns() {
    let tmp = tempdir().unwrap();
    let config_path = tmp.path().join(".spinelfs");

    let mut rules = LfsTrackingRules::new();
    rules.track_pattern("*.psd");
    rules.track_pattern("*.mp4");
    rules.track_pattern("assets/models/**");
    rules.set_threshold(Some(5 * 1024 * 1024)); // 5 MB

    rules.save_to_file(&config_path).unwrap();

    let loaded = LfsTrackingRules::load_from_file(&config_path).unwrap();
    assert_eq!(loaded.patterns.len(), 3);
    assert_eq!(loaded.size_threshold_bytes, Some(5 * 1024 * 1024));

    // Matching tests
    assert!(loaded.is_tracked("design/hero.psd", 100));
    assert!(loaded.is_tracked("videos/intro.mp4", 100));
    assert!(loaded.is_tracked("assets/models/character.mesh", 100));
    assert!(!loaded.is_tracked("src/main.rs", 100));

    // Size threshold match regardless of extension
    assert!(loaded.is_tracked("large_dataset.csv", 6 * 1024 * 1024));
    assert!(!loaded.is_tracked("small.csv", 1000));
}

#[test]
fn test_lfs_storage_chunking_and_reconstruction() {
    let tmp = tempdir().unwrap();
    let store_dir = tmp.path().join("store");
    let store = LocalArtifactStore::new(&store_dir).unwrap();

    // 8 MB file
    let file_size = 8 * 1024 * 1024;
    let data: Vec<u8> = (0..file_size).map(|i| (i % 251) as u8).collect();
    let original_hash = blake3::hash(&data).to_hex().to_string();

    let chunker = StreamChunker::new(Some(4 * 1024 * 1024));
    let chunk_res = chunker
        .chunk_reader(Cursor::new(&data), "huge_asset.bin", None)
        .unwrap();

    assert_eq!(chunk_res.chunks.len(), 2);
    assert_eq!(chunk_res.manifest.total_size, file_size as u64);
    assert_eq!(chunk_res.manifest.whole_object_hash, original_hash);

    // Store chunks and manifest
    for c in &chunk_res.chunks {
        store.store_chunk(&c.entry.chunk_hash, &c.bytes).unwrap();
    }
    let manifest_oid = store.store_manifest(&chunk_res.manifest).unwrap();

    // Verify pointer generation
    let ptr = LfsPointer::new(
        manifest_oid.clone(),
        chunk_res.manifest.total_size,
        chunk_res.manifest.whole_object_hash.clone(),
    );
    let pointer_str = ptr.encode();
    assert!(pointer_str.starts_with("spine-lfs-v1 "));

    // Hydrate back from store into buffer
    let mut hydrated = Vec::new();
    let retrieved_manifest = store.get_manifest(&manifest_oid).unwrap().unwrap();

    ObjectReconstructor::reconstruct(
        &retrieved_manifest,
        |idx| {
            let entry = retrieved_manifest.chunks.get(idx as usize).unwrap();
            let bytes = store.get_chunk(&entry.chunk_hash).unwrap().unwrap();
            Ok(bytes)
        },
        &mut hydrated,
    )
    .unwrap();

    assert_eq!(hydrated.len(), data.len());
    assert_eq!(hydrated, data);

    // Garbage collection verification
    let active_manifest_ids = vec![manifest_oid.clone()];
    let all_chunk_hashes: Vec<String> = chunk_res
        .chunks
        .iter()
        .map(|c| c.entry.chunk_hash.clone())
        .collect();

    let gc_report = ArtifactGcCollector::collect_garbage(
        &store,
        &active_manifest_ids,
        &all_chunk_hashes,
        true, // dry run
    )
    .unwrap();

    assert_eq!(gc_report.reachable_chunks, 2);
    assert_eq!(gc_report.candidate_chunks, 0);
}
