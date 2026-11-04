use anyhow::{bail, Context, Result};
use spine_artifact::{
    ArtifactGcCollector, ArtifactManifest, ArtifactStore, LfsPointer, LfsTrackingRules,
    LocalArtifactStore, ObjectReconstructor, StreamChunker, DEFAULT_CHUNK_SIZE,
};
use std::fs::{self, File};
use std::io::BufReader;
use std::path::{Path, PathBuf};

pub fn get_artifacts_dir(repo_root: &Path) -> PathBuf {
    repo_root.join(".spine").join("artifacts")
}

pub fn get_artifact_store(repo_root: &Path) -> Result<LocalArtifactStore> {
    let dir = get_artifacts_dir(repo_root);
    LocalArtifactStore::new(dir).context("Failed to open local artifact store")
}

/// Adds a pattern to .spinelfs
pub fn track(repo_root: &Path, pattern: &str) -> Result<()> {
    let spinelfs_path = repo_root.join(".spinelfs");
    let mut rules = if spinelfs_path.exists() {
        LfsTrackingRules::load_from_file(&spinelfs_path)
            .map_err(|e| anyhow::anyhow!("Failed to load .spinelfs: {}", e))?
    } else {
        LfsTrackingRules::default()
    };

    rules.track_pattern(pattern);
    rules
        .save_to_file(&spinelfs_path)
        .map_err(|e| anyhow::anyhow!("Failed to save .spinelfs: {}", e))?;

    println!("Tracking \"{}\" for Spine LFS", pattern);
    Ok(())
}

/// Removes a pattern from .spinelfs
pub fn untrack(repo_root: &Path, pattern: &str) -> Result<()> {
    let spinelfs_path = repo_root.join(".spinelfs");
    if !spinelfs_path.exists() {
        println!(
            "No .spinelfs file found; pattern \"{}\" was not tracked",
            pattern
        );
        return Ok(());
    }

    let mut rules = LfsTrackingRules::load_from_file(&spinelfs_path)
        .map_err(|e| anyhow::anyhow!("Failed to load .spinelfs: {}", e))?;

    let removed = rules.untrack_pattern(pattern);
    if removed {
        rules
            .save_to_file(&spinelfs_path)
            .map_err(|e| anyhow::anyhow!("Failed to save .spinelfs: {}", e))?;
        println!("Untracked \"{}\" from Spine LFS", pattern);
    } else {
        println!("Pattern \"{}\" was not found in .spinelfs", pattern);
    }
    Ok(())
}

/// Displays active tracking rules and cached storage statistics
pub fn status(repo_root: &Path) -> Result<()> {
    let spinelfs_path = repo_root.join(".spinelfs");
    if spinelfs_path.exists() {
        let rules = LfsTrackingRules::load_from_file(&spinelfs_path)
            .map_err(|e| anyhow::anyhow!("Failed to load .spinelfs: {}", e))?;
        println!("Spine LFS Tracking Rules (.spinelfs):");
        for p in &rules.patterns {
            println!("  - pattern: {}", p);
        }
        if let Some(threshold) = rules.size_threshold_bytes {
            println!("  - threshold: {} bytes", threshold);
        }
    } else {
        println!("No .spinelfs file found. (Run 'spine lfs track <pattern>' to start tracking)");
    }

    let artifacts_dir = get_artifacts_dir(repo_root);
    let chunks_dir = artifacts_dir.join("chunks");
    let manifests_dir = artifacts_dir.join("manifests");

    let mut chunk_count = 0usize;
    let mut chunk_bytes = 0u64;

    if chunks_dir.exists() {
        for entry in walkdir(chunks_dir)? {
            if entry.is_file() {
                chunk_count += 1;
                chunk_bytes += entry.metadata()?.len();
            }
        }
    }

    let mut manifest_count = 0usize;
    if manifests_dir.exists() {
        for entry in walkdir(manifests_dir)? {
            if entry.is_file() {
                manifest_count += 1;
            }
        }
    }

    println!("\nLocal Artifact Store (.spine/artifacts):");
    println!("  Manifests: {}", manifest_count);
    println!("  Stored chunks: {}", chunk_count);
    println!(
        "  Stored payload: {} bytes ({:.2} MiB)",
        chunk_bytes,
        chunk_bytes as f64 / (1024.0 * 1024.0)
    );

    Ok(())
}

/// Lists all LFS files currently in the working tree
pub fn ls_files(repo_root: &Path) -> Result<()> {
    let spinelfs_path = repo_root.join(".spinelfs");
    let rules = if spinelfs_path.exists() {
        LfsTrackingRules::load_from_file(&spinelfs_path)
            .map_err(|e| anyhow::anyhow!("Failed to load .spinelfs: {}", e))?
    } else {
        LfsTrackingRules::default()
    };

    println!(
        "{:<40} {:<12} {:<64} {}",
        "MANIFEST OID", "SIZE", "OBJECT HASH", "PATH"
    );
    println!("{}", "-".repeat(130));

    let mut count = 0;
    for entry in walk_tree_files(repo_root)? {
        let rel_path = entry.strip_prefix(repo_root)?;
        let rel_path_str = rel_path.to_string_lossy().replace('\\', "/");

        // Skip internal .spine directory
        if rel_path_str.starts_with(".spine") {
            continue;
        }

        let meta = fs::metadata(&entry)?;
        let is_tracked = rules.is_tracked(&rel_path_str, meta.len());

        // Check if file is already an LFS pointer
        if meta.len() < 1024 {
            if let Ok(content) = fs::read(&entry) {
                if let Ok(s) = std::str::from_utf8(&content) {
                    if let Ok(ptr) = LfsPointer::parse(s) {
                        println!(
                            "{:<40} {:<12} {:<64} {}",
                            ptr.manifest_oid, ptr.size_bytes, ptr.object_hash, rel_path_str
                        );
                        count += 1;
                        continue;
                    }
                }
            }
        }

        if is_tracked {
            println!(
                "{:<40} {:<12} {:<64} {} (uncommitted payload)",
                "-",
                meta.len(),
                "-",
                rel_path_str
            );
            count += 1;
        }
    }

    if count == 0 {
        println!("No LFS files found.");
    }

    Ok(())
}

/// Verifies integrity of all manifests and chunks in local store
pub fn verify(repo_root: &Path) -> Result<()> {
    let store = get_artifact_store(repo_root)?;
    let manifests_dir = get_artifacts_dir(repo_root).join("manifests");

    if !manifests_dir.exists() {
        println!("No manifests found in local store.");
        return Ok(());
    }

    let mut verified = 0;
    let mut errors = 0;

    for entry in walkdir(manifests_dir)? {
        if entry.is_file() && entry.extension().map_or(false, |ext| ext == "json") {
            let content = fs::read_to_string(&entry)?;
            let manifest: ArtifactManifest = match serde_json::from_str(&content) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Corrupted manifest JSON at {:?}: {}", entry, e);
                    errors += 1;
                    continue;
                }
            };

            // Verify chunk availability & integrity
            let mut missing = 0;
            let mut corrupt = 0;

            for chunk_entry in &manifest.chunks {
                match store.get_chunk(&chunk_entry.chunk_hash) {
                    Ok(Some(bytes)) => {
                        let actual = blake3::hash(&bytes).to_hex().to_string();
                        if actual != chunk_entry.chunk_hash {
                            corrupt += 1;
                        }
                    }
                    Ok(None) => missing += 1,
                    Err(_) => corrupt += 1,
                }
            }

            if missing == 0 && corrupt == 0 {
                println!(
                    "OK: manifest {} ({} chunks, {} bytes)",
                    manifest.artifact_id,
                    manifest.chunks.len(),
                    manifest.total_size
                );
                verified += 1;
            } else {
                eprintln!(
                    "FAILED: manifest {} — missing: {}, corrupt: {}",
                    manifest.artifact_id, missing, corrupt
                );
                errors += 1;
            }
        }
    }

    println!(
        "\nVerification summary: {} verified, {} errors",
        verified, errors
    );
    if errors > 0 {
        bail!("Artifact verification completed with errors");
    }
    Ok(())
}

/// Performs garbage collection on unreferenced chunks
pub fn prune(repo_root: &Path, dry_run: bool) -> Result<()> {
    let store = get_artifact_store(repo_root)?;
    let artifacts_dir = get_artifacts_dir(repo_root);

    let active_manifest_ids = load_all_manifest_ids(&artifacts_dir)?;
    let all_chunk_hashes = load_all_chunk_hashes(&artifacts_dir)?;
    let report = ArtifactGcCollector::collect_garbage(
        &store,
        &active_manifest_ids,
        &all_chunk_hashes,
        dry_run,
    )
    .map_err(|e| anyhow::anyhow!("GC collection failed: {}", e))?;

    println!("Spine LFS Garbage Collection Report:");
    println!("  Active manifests: {}", active_manifest_ids.len());
    println!("  Total chunks scanned: {}", report.total_chunks_scanned);
    println!("  Reachable chunks: {}", report.reachable_chunks);
    println!("  Candidate chunks: {}", report.candidate_chunks);
    println!(
        "  Reclaimed space: {} bytes ({:.2} MiB)",
        report.reclaimed_bytes,
        report.reclaimed_bytes as f64 / (1024.0 * 1024.0)
    );
    if dry_run {
        println!("  (Dry run: no chunks were deleted)");
    }
    Ok(())
}

pub fn resolve_remote_store(repo_root: &Path, remote: &str) -> Result<LocalArtifactStore> {
    let spine_dir = repo_root.join(".spine");
    let config_path = spine_dir.join("spine.toml");
    let url = if config_path.exists() {
        if let Ok(config) = spine_config::SpineConfig::load_from_file(&config_path) {
            config
                .get_remote(remote)
                .map(|r| r.url.clone())
                .unwrap_or_else(|| remote.to_string())
        } else {
            remote.to_string()
        }
    } else {
        remote.to_string()
    };

    let path_str = url.strip_prefix("file://").unwrap_or(&url);
    let path = PathBuf::from(path_str);
    let artifacts_dir = if path.join(".spine").exists() {
        path.join(".spine").join("artifacts")
    } else if path.join("artifacts").exists() || path.join("chunks").exists() {
        path.join("artifacts")
    } else {
        path.join(".spine").join("artifacts")
    };
    LocalArtifactStore::new(artifacts_dir).context("Failed to open remote artifact store")
}

/// Pushes local LFS manifests and chunks to the remote artifact store
pub fn push(repo_root: &Path, remote: &str) -> Result<()> {
    let local_store = get_artifact_store(repo_root)?;
    let remote_store = resolve_remote_store(repo_root, remote)?;
    let artifacts_dir = get_artifacts_dir(repo_root);

    let manifest_ids = load_all_manifest_ids(&artifacts_dir)?;
    let mut pushed_manifests = 0;
    let mut pushed_chunks = 0;
    let mut pushed_bytes = 0u64;

    for manifest_id in &manifest_ids {
        if let Ok(Some(manifest)) = local_store.get_manifest(manifest_id) {
            for chunk in &manifest.chunks {
                if !remote_store.has_chunk(&chunk.chunk_hash) {
                    if let Ok(Some(bytes)) = local_store.get_chunk(&chunk.chunk_hash) {
                        remote_store.store_chunk(&chunk.chunk_hash, &bytes)?;
                        pushed_chunks += 1;
                        pushed_bytes += bytes.len() as u64;
                    }
                }
            }
            if !remote_store.has_manifest(manifest_id) {
                remote_store.store_manifest(&manifest)?;
                pushed_manifests += 1;
            }
        }
    }

    println!(
        "LFS push complete: {} manifests, {} chunks ({} bytes) pushed to '{}'",
        pushed_manifests, pushed_chunks, pushed_bytes, remote
    );
    Ok(())
}

/// Fetches referenced LFS manifests and chunks from the remote artifact store
pub fn fetch(repo_root: &Path, remote: &str) -> Result<()> {
    let local_store = get_artifact_store(repo_root)?;
    let remote_store = resolve_remote_store(repo_root, remote)?;

    let mut fetched_manifests = 0;
    let mut fetched_chunks = 0;
    let mut fetched_bytes = 0u64;

    for entry in walk_tree_files(repo_root)? {
        let rel_path = match entry.strip_prefix(repo_root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        if rel_str.starts_with(".spine") {
            continue;
        }

        if let Ok(content) = fs::read(&entry) {
            if let Ok(s) = std::str::from_utf8(&content) {
                if let Ok(ptr) = LfsPointer::parse(s) {
                    if !local_store.has_manifest(&ptr.manifest_oid) {
                        if let Ok(Some(m)) = remote_store.get_manifest(&ptr.manifest_oid) {
                            local_store.store_manifest(&m)?;
                            fetched_manifests += 1;
                        }
                    }

                    if let Ok(Some(m)) = local_store.get_manifest(&ptr.manifest_oid) {
                        for chunk in &m.chunks {
                            if !local_store.has_chunk(&chunk.chunk_hash) {
                                if let Ok(Some(bytes)) = remote_store.get_chunk(&chunk.chunk_hash) {
                                    local_store.store_chunk(&chunk.chunk_hash, &bytes)?;
                                    fetched_chunks += 1;
                                    fetched_bytes += bytes.len() as u64;
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    println!(
        "LFS fetch complete: {} manifests, {} chunks ({} bytes) fetched from '{}'",
        fetched_manifests, fetched_chunks, fetched_bytes, remote
    );
    Ok(())
}

/// Pulls LFS manifests and chunks from remote and hydrates all pointers in working directory
pub fn pull(repo_root: &Path, remote: &str) -> Result<()> {
    fetch(repo_root, remote)?;

    let mut hydrated = 0;
    for entry in walk_tree_files(repo_root)? {
        let rel_path = match entry.strip_prefix(repo_root) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        if rel_str.starts_with(".spine") {
            continue;
        }

        if let Ok(content) = fs::read(&entry) {
            if hydrate_pointer_file(repo_root, &entry, &content)? {
                hydrated += 1;
            }
        }
    }

    println!(
        "LFS pull: Hydrated {} large files in working tree",
        hydrated
    );
    Ok(())
}

/// Intercepts a file during `spine add`: if it matches LFS tracking rules,
/// chunks it into `.spine/artifacts/`, writes chunks + manifest,
/// and returns the serialized `LfsPointer` bytes to be stored in the index/tree!
pub fn process_lfs_add(
    repo_root: &Path,
    rel_path: &str,
    full_path: &Path,
) -> Result<Option<Vec<u8>>> {
    let spinelfs_path = repo_root.join(".spinelfs");
    if !spinelfs_path.exists() {
        return Ok(None);
    }

    let rules = LfsTrackingRules::load_from_file(&spinelfs_path)
        .map_err(|e| anyhow::anyhow!("Failed to load .spinelfs: {}", e))?;

    let meta = fs::metadata(full_path)?;
    if !rules.is_tracked(rel_path, meta.len()) {
        return Ok(None);
    }

    // Check if it's already an LFS pointer
    if meta.len() < 1024 {
        let content = fs::read(full_path)?;
        if LfsPointer::is_lfs_pointer_bytes(&content) {
            return Ok(Some(content));
        }
    }

    // Chunk and store in local artifact store
    let store = get_artifact_store(repo_root)?;
    let file = File::open(full_path)?;
    let reader = BufReader::new(file);

    let chunker = StreamChunker::new(Some(DEFAULT_CHUNK_SIZE));
    let chunk_result = chunker
        .chunk_reader(reader, rel_path, None)
        .map_err(|e| anyhow::anyhow!("Streaming chunker failed: {}", e))?;

    // Store chunks
    for chunk in &chunk_result.chunks {
        store
            .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
            .map_err(|e| anyhow::anyhow!("Failed to store chunk: {}", e))?;
    }

    // Store manifest
    let manifest_oid = store
        .store_manifest(&chunk_result.manifest)
        .map_err(|e| anyhow::anyhow!("Failed to store manifest: {}", e))?;

    let pointer = LfsPointer::new(
        manifest_oid,
        chunk_result.manifest.total_size,
        chunk_result.manifest.whole_object_hash,
    );

    let pointer_bytes = pointer.encode().into_bytes();
    Ok(Some(pointer_bytes))
}

/// Hydrates a file in the working tree if it is an LFS pointer and the artifact exists locally
pub fn hydrate_pointer_file(repo_root: &Path, target_path: &Path, content: &[u8]) -> Result<bool> {
    let s = match std::str::from_utf8(content) {
        Ok(v) => v,
        Err(_) => return Ok(false),
    };
    let ptr = match LfsPointer::parse(s) {
        Ok(p) => p,
        Err(_) => return Ok(false),
    };

    let store = get_artifact_store(repo_root)?;
    let manifest = match store.get_manifest(&ptr.manifest_oid) {
        Ok(Some(m)) => m,
        _ => return Ok(false), // Manifest not present locally, keep pointer
    };

    // Check if all chunks are available
    for entry in &manifest.chunks {
        if !store.has_chunk(&entry.chunk_hash) {
            return Ok(false);
        }
    }

    // Reconstruct directly into target file
    let tmp_path = target_path.with_extension("tmp_lfs");
    let mut out_file = File::create(&tmp_path)?;

    ObjectReconstructor::reconstruct(
        &manifest,
        |idx| {
            let entry = manifest
                .chunks
                .get(idx as usize)
                .ok_or_else(|| spine_artifact::chunker::ChunkerError::MissingChunk(idx))?;
            let bytes = store
                .get_chunk(&entry.chunk_hash)
                .map_err(|e| {
                    spine_artifact::chunker::ChunkerError::Io(std::io::Error::new(
                        std::io::ErrorKind::Other,
                        e.to_string(),
                    ))
                })?
                .ok_or_else(|| spine_artifact::chunker::ChunkerError::MissingChunk(idx))?;
            Ok(bytes)
        },
        &mut out_file,
    )
    .map_err(|e| anyhow::anyhow!("LFS reconstruction failed: {}", e))?;

    fs::rename(tmp_path, target_path)?;
    Ok(true)
}

fn load_all_manifest_ids(artifacts_dir: &Path) -> Result<Vec<String>> {
    let manifests_dir = artifacts_dir.join("manifests");
    let mut ids = Vec::new();

    if manifests_dir.exists() {
        for entry in walkdir(manifests_dir)? {
            if entry.is_file() && entry.extension().map_or(false, |ext| ext == "json") {
                if let Some(stem) = entry.file_stem().and_then(|s| s.to_str()) {
                    ids.push(stem.to_string());
                }
            }
        }
    }
    Ok(ids)
}

fn load_all_chunk_hashes(artifacts_dir: &Path) -> Result<Vec<String>> {
    let chunks_dir = artifacts_dir.join("chunks");
    let mut hashes = Vec::new();

    if chunks_dir.exists() {
        for entry in walkdir(chunks_dir)? {
            if entry.is_file() {
                if let Some(stem) = entry.file_name().and_then(|s| s.to_str()) {
                    hashes.push(stem.to_string());
                }
            }
        }
    }
    Ok(hashes)
}

fn walkdir(dir: PathBuf) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                results.extend(walkdir(path)?);
            } else {
                results.push(path);
            }
        }
    }
    Ok(results)
}

fn walk_tree_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut results = Vec::new();
    if dir.is_dir() {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                results.extend(walk_tree_files(&path)?);
            } else {
                results.push(path);
            }
        }
    }
    Ok(results)
}
