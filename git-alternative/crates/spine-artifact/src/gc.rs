use crate::store::ArtifactStore;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use thiserror::Error;

pub const DEFAULT_ARTIFACT_GC_GRACE_SECS: i64 = 86400; // 24 hours

#[derive(Error, Debug)]
pub enum GcError {
    #[error("Store error: {0}")]
    Store(#[from] crate::store::StoreError),
}

/// An active in-flight upload lease protecting newly written CAS chunks from GC
/// even if the upload duration exceeds filesystem mtime grace intervals.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct InFlightUploadLease {
    pub lease_id: String,
    pub created_at_epoch_secs: i64,
    pub expires_at_epoch_secs: i64,
    pub pinned_chunks: HashSet<String>,
}

fn write_disk_lease(dir: &Path, lease: &InFlightUploadLease) {
    if std::fs::create_dir_all(dir).is_ok() {
        let tmp_path = dir.join(format!("{}.lease.tmp", lease.lease_id));
        let final_path = dir.join(format!("{}.lease", lease.lease_id));
        if let Ok(bytes) = serde_json::to_vec(lease) {
            if std::fs::write(&tmp_path, bytes).is_ok() {
                let _ = std::fs::rename(&tmp_path, &final_path);
            }
        }
    }
}

fn remove_disk_lease(dir: &Path, lease_id: &str) {
    let final_path = dir.join(format!("{}.lease", lease_id));
    let tmp_path = dir.join(format!("{}.lease.tmp", lease_id));
    let _ = std::fs::remove_file(final_path);
    let _ = std::fs::remove_file(tmp_path);
}

/// Registry for active in-flight upload leases. Thread-safe and queryable by GC.
/// Supports both in-memory leases and shared durable filesystem leases across worker processes.
#[derive(Debug, Default)]
pub struct InFlightUploadRegistry {
    leases: RwLock<HashMap<String, InFlightUploadLease>>,
    shared_dir: Option<PathBuf>,
}

impl InFlightUploadRegistry {
    pub fn new() -> Self {
        Self {
            leases: RwLock::new(HashMap::new()),
            shared_dir: None,
        }
    }

    pub fn with_shared_dir(shared_dir: Option<PathBuf>) -> Self {
        Self {
            leases: RwLock::new(HashMap::new()),
            shared_dir,
        }
    }

    pub fn shared_dir(&self) -> Option<&Path> {
        self.shared_dir.as_deref()
    }

    /// Register a new in-flight upload lease with a specified TTL in seconds.
    pub fn register_lease(&self, lease_id: &str, ttl_secs: i64) -> InFlightUploadLease {
        let now = chrono::Utc::now().timestamp();
        let lease = InFlightUploadLease {
            lease_id: lease_id.to_string(),
            created_at_epoch_secs: now,
            expires_at_epoch_secs: now + ttl_secs.max(1),
            pinned_chunks: HashSet::new(),
        };
        {
            let mut map = self.leases.write().unwrap();
            map.insert(lease_id.to_string(), lease.clone());
        }
        if let Some(ref dir) = self.shared_dir {
            write_disk_lease(dir, &lease);
        }
        lease
    }

    /// Pin a chunk hash under an active lease.
    pub fn pin_chunk(&self, lease_id: &str, chunk_hash: &str) -> bool {
        let updated_lease = {
            let mut map = self.leases.write().unwrap();
            if let Some(lease) = map.get_mut(lease_id) {
                lease.pinned_chunks.insert(chunk_hash.to_string());
                Some(lease.clone())
            } else {
                None
            }
        };
        if let Some(ref lease) = updated_lease {
            if let Some(ref dir) = self.shared_dir {
                write_disk_lease(dir, lease);
            }
            true
        } else {
            false
        }
    }

    /// Pin multiple chunk hashes under an active lease.
    pub fn pin_chunks(&self, lease_id: &str, chunk_hashes: &[String]) -> bool {
        let updated_lease = {
            let mut map = self.leases.write().unwrap();
            if let Some(lease) = map.get_mut(lease_id) {
                for ch in chunk_hashes {
                    lease.pinned_chunks.insert(ch.clone());
                }
                Some(lease.clone())
            } else {
                None
            }
        };
        if let Some(ref lease) = updated_lease {
            if let Some(ref dir) = self.shared_dir {
                write_disk_lease(dir, lease);
            }
            true
        } else {
            false
        }
    }

    /// Extend lease expiration (heartbeat).
    pub fn heartbeat(&self, lease_id: &str, extend_secs: i64) -> bool {
        let now = chrono::Utc::now().timestamp();
        let updated_lease = {
            let mut map = self.leases.write().unwrap();
            if let Some(lease) = map.get_mut(lease_id) {
                lease.expires_at_epoch_secs = now + extend_secs.max(1);
                Some(lease.clone())
            } else {
                None
            }
        };
        if let Some(ref lease) = updated_lease {
            if let Some(ref dir) = self.shared_dir {
                write_disk_lease(dir, lease);
            }
            true
        } else {
            false
        }
    }

    /// Complete an upload lease upon successful manifest commit.
    pub fn complete_lease(&self, lease_id: &str) {
        {
            let mut map = self.leases.write().unwrap();
            map.remove(lease_id);
        }
        if let Some(ref dir) = self.shared_dir {
            remove_disk_lease(dir, lease_id);
        }
    }

    /// Cancel an upload lease upon client abort or error.
    pub fn cancel_lease(&self, lease_id: &str) {
        {
            let mut map = self.leases.write().unwrap();
            map.remove(lease_id);
        }
        if let Some(ref dir) = self.shared_dir {
            remove_disk_lease(dir, lease_id);
        }
    }

    /// Prune expired leases and return the count of pruned leases.
    pub fn purge_expired_leases(&self, now_secs: i64) -> usize {
        let mut purged = {
            let mut map = self.leases.write().unwrap();
            let initial_len = map.len();
            map.retain(|_, lease| lease.expires_at_epoch_secs > now_secs);
            initial_len - map.len()
        };
        if let Some(ref dir) = self.shared_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("lease") {
                        if let Ok(bytes) = std::fs::read(&path) {
                            if let Ok(lease) = serde_json::from_slice::<InFlightUploadLease>(&bytes)
                            {
                                if lease.expires_at_epoch_secs <= now_secs {
                                    let _ = std::fs::remove_file(&path);
                                    purged += 1;
                                }
                            }
                        }
                    }
                }
            }
        }
        purged
    }

    /// Get all chunk hashes currently protected across active (unexpired) leases.
    pub fn get_active_pinned_chunks(&self, now_secs: i64) -> HashSet<String> {
        let mut pinned = HashSet::new();
        {
            let mut map = self.leases.write().unwrap();
            map.retain(|_, lease| lease.expires_at_epoch_secs > now_secs);
            for lease in map.values() {
                pinned.extend(lease.pinned_chunks.iter().cloned());
            }
        }
        if let Some(ref dir) = self.shared_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("lease") {
                        if let Ok(bytes) = std::fs::read(&path) {
                            if let Ok(lease) = serde_json::from_slice::<InFlightUploadLease>(&bytes)
                            {
                                if lease.expires_at_epoch_secs > now_secs {
                                    pinned.extend(lease.pinned_chunks);
                                } else {
                                    let _ = std::fs::remove_file(&path);
                                }
                            }
                        }
                    }
                }
            }
        }
        pinned
    }

    /// Check if a specific chunk is pinned in any active lease.
    pub fn is_chunk_pinned(&self, chunk_hash: &str, now_secs: i64) -> bool {
        {
            let mut map = self.leases.write().unwrap();
            map.retain(|_, lease| lease.expires_at_epoch_secs > now_secs);
            if map
                .values()
                .any(|lease| lease.pinned_chunks.contains(chunk_hash))
            {
                return true;
            }
        }
        if let Some(ref dir) = self.shared_dir {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().and_then(|e| e.to_str()) == Some("lease") {
                        if let Ok(bytes) = std::fs::read(&path) {
                            if let Ok(lease) = serde_json::from_slice::<InFlightUploadLease>(&bytes)
                            {
                                if lease.expires_at_epoch_secs > now_secs
                                    && lease.pinned_chunks.contains(chunk_hash)
                                {
                                    return true;
                                }
                            }
                        }
                    }
                }
            }
        }
        false
    }
}

#[derive(Clone, Debug, Default)]
pub struct ArtifactGcReport {
    pub total_chunks_scanned: usize,
    pub reachable_chunks: usize,
    pub candidate_chunks: usize,
    pub grace_protected_chunks: usize,
    pub in_flight_protected_chunks: usize,
    pub reclaimed_bytes: u64,
    pub dry_run: bool,
}

pub struct ArtifactGcCollector;

impl ArtifactGcCollector {
    /// Computes reachable chunks given a set of active manifest IDs.
    pub fn trace_reachable_chunks<S: ArtifactStore + ?Sized>(
        store: &S,
        active_manifest_ids: &[String],
    ) -> Result<HashSet<String>, GcError> {
        let mut reachable = HashSet::new();

        for m_id in active_manifest_ids {
            if let Some(manifest) = store.get_manifest(m_id)? {
                for chunk in &manifest.chunks {
                    reachable.insert(chunk.chunk_hash.clone());
                }
            }
        }

        Ok(reachable)
    }

    /// Generates a GC report and optionally removes unreachable chunks not protected by grace period.
    pub fn collect_garbage<S: ArtifactStore + ?Sized>(
        store: &S,
        active_manifest_ids: &[String],
        all_chunk_hashes: &[String],
        dry_run: bool,
    ) -> Result<ArtifactGcReport, GcError> {
        Self::collect_garbage_with_registry(
            store,
            active_manifest_ids,
            all_chunk_hashes,
            None,
            0,
            dry_run,
        )
    }

    /// Generates a GC report with configurable grace period protection for in-flight/recent chunks.
    pub fn collect_garbage_with_grace<S: ArtifactStore + ?Sized>(
        store: &S,
        active_manifest_ids: &[String],
        all_chunk_hashes: &[String],
        grace_secs: i64,
        dry_run: bool,
    ) -> Result<ArtifactGcReport, GcError> {
        Self::collect_garbage_with_registry(
            store,
            active_manifest_ids,
            all_chunk_hashes,
            None,
            grace_secs,
            dry_run,
        )
    }

    /// Generates a GC report with in-flight registry lease protection and mtime grace period protection.
    pub fn collect_garbage_with_registry<S: ArtifactStore + ?Sized>(
        store: &S,
        active_manifest_ids: &[String],
        all_chunk_hashes: &[String],
        in_flight_registry: Option<&InFlightUploadRegistry>,
        grace_secs: i64,
        dry_run: bool,
    ) -> Result<ArtifactGcReport, GcError> {
        let reachable = Self::trace_reachable_chunks(store, active_manifest_ids)?;
        let now = chrono::Utc::now().timestamp();
        let in_flight_pinned = in_flight_registry
            .map(|reg| reg.get_active_pinned_chunks(now))
            .unwrap_or_default();

        let mut report = ArtifactGcReport {
            total_chunks_scanned: all_chunk_hashes.len(),
            reachable_chunks: 0,
            candidate_chunks: 0,
            grace_protected_chunks: 0,
            in_flight_protected_chunks: 0,
            reclaimed_bytes: 0,
            dry_run,
        };

        for chunk_hash in all_chunk_hashes {
            if reachable.contains(chunk_hash) {
                report.reachable_chunks += 1;
            } else if in_flight_pinned.contains(chunk_hash) {
                report.in_flight_protected_chunks += 1;
            } else {
                // If chunk was modified within grace period, protect it from GC (e.g. recent disk write)
                if grace_secs > 0 {
                    if let Ok(Some(mtime)) = store.get_chunk_mtime_secs(chunk_hash) {
                        if now - mtime < grace_secs {
                            report.grace_protected_chunks += 1;
                            continue;
                        }
                    }
                }

                report.candidate_chunks += 1;
                if let Ok(Some(bytes)) = store.get_chunk(chunk_hash) {
                    report.reclaimed_bytes += bytes.len() as u64;
                }
            }
        }

        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ArtifactManifest, ChunkEntry};
    use crate::store::LocalArtifactStore;
    use tempfile::TempDir;

    #[test]
    fn test_gc_reachability_and_preservation() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();

        // 1. Create chunk 1 and chunk 2
        let chunk1_bytes = b"chunk 1 content";
        let chunk1_hash = blake3::hash(chunk1_bytes).to_hex().to_string();
        store.store_chunk(&chunk1_hash, chunk1_bytes).unwrap();

        let chunk2_bytes = b"chunk 2 orphan content";
        let chunk2_hash = blake3::hash(chunk2_bytes).to_hex().to_string();
        store.store_chunk(&chunk2_hash, chunk2_bytes).unwrap();

        // 2. Create manifest referencing only chunk 1
        let manifest = ArtifactManifest::new(
            "art_live".into(),
            chunk1_hash.clone(),
            chunk1_bytes.len() as u64,
            chunk1_bytes.len() as u32,
            vec![ChunkEntry {
                index: 0,
                offset: 0,
                length: chunk1_bytes.len() as u32,
                chunk_hash: chunk1_hash.clone(),
            }],
            None,
        )
        .unwrap();

        let manifest_id = store.store_manifest(&manifest).unwrap();

        // 3. Run GC
        let all_chunks = vec![chunk1_hash.clone(), chunk2_hash.clone()];
        let report = ArtifactGcCollector::collect_garbage(
            &store,
            &[manifest_id],
            &all_chunks,
            true, // dry run
        )
        .unwrap();

        assert_eq!(report.reachable_chunks, 1);
        assert_eq!(report.candidate_chunks, 1);
        assert_eq!(report.reclaimed_bytes, chunk2_bytes.len() as u64);
    }

    #[test]
    fn test_gc_multi_manifest_shared_chunks_and_partial_deref() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();

        // 4 chunks: A, B (shared), C, D
        let chunk_a = b"chunk A unique to release";
        let hash_a = blake3::hash(chunk_a).to_hex().to_string();
        store.store_chunk(&hash_a, chunk_a).unwrap();

        let chunk_b = b"chunk B shared between release and package";
        let hash_b = blake3::hash(chunk_b).to_hex().to_string();
        store.store_chunk(&hash_b, chunk_b).unwrap();

        let chunk_c = b"chunk C unique to package";
        let hash_c = blake3::hash(chunk_c).to_hex().to_string();
        store.store_chunk(&hash_c, chunk_c).unwrap();

        let chunk_d = b"chunk D unique to LFS";
        let hash_d = blake3::hash(chunk_d).to_hex().to_string();
        store.store_chunk(&hash_d, chunk_d).unwrap();

        // Manifest 1 (Release): [Chunk A, Chunk B]
        let m1 = ArtifactManifest::new(
            "release_v1".into(),
            "hash_rel".into(),
            (chunk_a.len() + chunk_b.len()) as u64,
            chunk_a.len() as u32,
            vec![
                ChunkEntry {
                    index: 0,
                    offset: 0,
                    length: chunk_a.len() as u32,
                    chunk_hash: hash_a.clone(),
                },
                ChunkEntry {
                    index: 1,
                    offset: chunk_a.len() as u64,
                    length: chunk_b.len() as u32,
                    chunk_hash: hash_b.clone(),
                },
            ],
            None,
        )
        .unwrap();
        let m1_id = store.store_manifest(&m1).unwrap();

        // Manifest 2 (Cargo Crate): [Chunk B, Chunk C]
        let m2 = ArtifactManifest::new(
            "cargo_crate".into(),
            "hash_pkg".into(),
            (chunk_b.len() + chunk_c.len()) as u64,
            chunk_b.len() as u32,
            vec![
                ChunkEntry {
                    index: 0,
                    offset: 0,
                    length: chunk_b.len() as u32,
                    chunk_hash: hash_b.clone(),
                },
                ChunkEntry {
                    index: 1,
                    offset: chunk_b.len() as u64,
                    length: chunk_c.len() as u32,
                    chunk_hash: hash_c.clone(),
                },
            ],
            None,
        )
        .unwrap();
        let m2_id = store.store_manifest(&m2).unwrap();

        // Manifest 3 (LFS Pointer): [Chunk D]
        let m3 = ArtifactManifest::new(
            "lfs_weights".into(),
            "hash_lfs".into(),
            chunk_d.len() as u64,
            chunk_d.len() as u32,
            vec![ChunkEntry {
                index: 0,
                offset: 0,
                length: chunk_d.len() as u32,
                chunk_hash: hash_d.clone(),
            }],
            None,
        )
        .unwrap();
        let m3_id = store.store_manifest(&m3).unwrap();

        let all_chunks = vec![
            hash_a.clone(),
            hash_b.clone(),
            hash_c.clone(),
            hash_d.clone(),
        ];

        // Phase 1: All 3 manifests active -> all 4 chunks reachable (0 candidates)
        let rep1 = ArtifactGcCollector::collect_garbage(
            &store,
            &[m1_id.clone(), m2_id.clone(), m3_id.clone()],
            &all_chunks,
            false,
        )
        .unwrap();
        assert_eq!(rep1.reachable_chunks, 4);
        assert_eq!(rep1.candidate_chunks, 0);

        // Phase 2: Release deleted (m1 omitted). m2 and m3 active.
        // Chunk A becomes orphan.
        // Chunk B MUST remain preserved because m2 still references it!
        let rep2 = ArtifactGcCollector::collect_garbage(
            &store,
            &[m2_id.clone(), m3_id.clone()],
            &all_chunks,
            false,
        )
        .unwrap();
        assert_eq!(rep2.reachable_chunks, 3); // B, C, D
        assert_eq!(rep2.candidate_chunks, 1); // Only A is candidate
        assert_eq!(rep2.reclaimed_bytes, chunk_a.len() as u64);

        // Phase 3: Cargo crate also deleted (m2 omitted). Only m3 active.
        // Chunks A, B, C become candidates. Only D is preserved.
        let rep3 = ArtifactGcCollector::collect_garbage(
            &store,
            std::slice::from_ref(&m3_id),
            &all_chunks,
            false,
        )
        .unwrap();
        assert_eq!(rep3.reachable_chunks, 1); // Only D
        assert_eq!(rep3.candidate_chunks, 3); // A, B, C
        assert_eq!(
            rep3.reclaimed_bytes,
            (chunk_a.len() + chunk_b.len() + chunk_c.len()) as u64
        );
    }

    #[test]
    fn test_gc_in_flight_upload_lease_protects_longer_than_grace_interval() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();
        let registry = InFlightUploadRegistry::new();

        // 1. In-flight upload starts and registers lease with 60s TTL
        let lease_id = "upload_slow_stream_001";
        registry.register_lease(lease_id, 60);

        let chunk_slow = b"chunk from slow upload staying active longer than grace";
        let hash_slow = blake3::hash(chunk_slow).to_hex().to_string();
        store.store_chunk(&hash_slow, chunk_slow).unwrap();

        // Pin chunk in the in-flight lease
        assert!(registry.pin_chunk(lease_id, &hash_slow));

        let all_chunks = vec![hash_slow.clone()];

        // 2. Simulate GC running repeatedly with a very short grace period (0s grace).
        // Since the chunk has no manifest, mtime grace alone would collect it if grace=0.
        // But with in_flight_registry, it is protected!
        for _ in 0..5 {
            let report = ArtifactGcCollector::collect_garbage_with_registry(
                &store,
                &[], // no active manifests yet!
                &all_chunks,
                Some(&registry),
                0, // 0s grace
                false,
            )
            .unwrap();

            assert_eq!(report.reachable_chunks, 0);
            assert_eq!(report.candidate_chunks, 0);
            assert_eq!(report.in_flight_protected_chunks, 1);
            assert_eq!(report.reclaimed_bytes, 0);
        }

        // 3. Complete the upload: create manifest referencing the chunk, then complete lease
        let manifest = ArtifactManifest::new(
            "art_slow".into(),
            hash_slow.clone(),
            chunk_slow.len() as u64,
            chunk_slow.len() as u32,
            vec![ChunkEntry {
                index: 0,
                offset: 0,
                length: chunk_slow.len() as u32,
                chunk_hash: hash_slow.clone(),
            }],
            None,
        )
        .unwrap();
        let m_id = store.store_manifest(&manifest).unwrap();
        registry.complete_lease(lease_id);

        // 4. Run GC again: now chunk is reachable via manifest!
        let report_after = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[m_id],
            &all_chunks,
            Some(&registry),
            0,
            false,
        )
        .unwrap();
        assert_eq!(report_after.reachable_chunks, 1);
        assert_eq!(report_after.candidate_chunks, 0);
        assert_eq!(report_after.in_flight_protected_chunks, 0);
    }

    #[test]
    fn test_gc_in_flight_upload_cancellation_and_orphan_collection() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();
        let registry = InFlightUploadRegistry::new();

        let lease_id = "upload_cancelled_002";
        registry.register_lease(lease_id, 30);

        let chunk_bytes = b"chunk from aborted upload";
        let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();
        store.store_chunk(&chunk_hash, chunk_bytes).unwrap();
        registry.pin_chunk(lease_id, &chunk_hash);

        let all_chunks = vec![chunk_hash.clone()];

        // While active, chunk is protected
        let rep1 = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[],
            &all_chunks,
            Some(&registry),
            0,
            false,
        )
        .unwrap();
        assert_eq!(rep1.in_flight_protected_chunks, 1);
        assert_eq!(rep1.candidate_chunks, 0);

        // Client aborts/cancels upload
        registry.cancel_lease(lease_id);

        // GC runs: chunk is immediately candidate orphan and reclaimed
        let rep2 = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[],
            &all_chunks,
            Some(&registry),
            0,
            false,
        )
        .unwrap();
        assert_eq!(rep2.in_flight_protected_chunks, 0);
        assert_eq!(rep2.candidate_chunks, 1);
        assert_eq!(rep2.reclaimed_bytes, chunk_bytes.len() as u64);
    }

    #[test]
    fn test_gc_in_flight_upload_crash_expiry_and_cleanup() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();
        let registry = InFlightUploadRegistry::new();

        let lease_id = "upload_crashed_003";
        // Lease with 1 second TTL simulating crashed worker that stopped renewing
        registry.register_lease(lease_id, 1);

        let chunk_bytes = b"chunk from crashed process";
        let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();
        store.store_chunk(&chunk_hash, chunk_bytes).unwrap();
        registry.pin_chunk(lease_id, &chunk_hash);

        // Sleep 1.1s so lease expires
        std::thread::sleep(std::time::Duration::from_millis(1100));

        let all_chunks = vec![chunk_hash.clone()];

        // GC runs: detects lease expired, purges it, and marks chunk as candidate
        let report = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[],
            &all_chunks,
            Some(&registry),
            0,
            false,
        )
        .unwrap();
        assert_eq!(report.in_flight_protected_chunks, 0);
        assert_eq!(report.candidate_chunks, 1);
        assert_eq!(report.reclaimed_bytes, chunk_bytes.len() as u64);
    }

    #[test]
    fn test_gc_in_flight_upload_coarse_timestamp_and_clock_skew_resilience() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();
        let registry = InFlightUploadRegistry::new();

        let lease_id = "upload_skew_004";
        registry.register_lease(lease_id, 120);

        let chunk_bytes = b"chunk with coarse mtime";
        let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();
        store.store_chunk(&chunk_hash, chunk_bytes).unwrap();
        registry.pin_chunk(lease_id, &chunk_hash);

        let all_chunks = vec![chunk_hash.clone()];

        // Even with grace_secs = 0 and simulated elapsed time, registry protects chunk
        let report = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[],
            &all_chunks,
            Some(&registry),
            0,
            false,
        )
        .unwrap();
        assert_eq!(report.in_flight_protected_chunks, 1);
        assert_eq!(report.candidate_chunks, 0);
    }

    #[test]
    fn test_gc_in_flight_shared_durable_leases_protect_across_separate_workers() {
        let tmp = TempDir::new().unwrap();
        let store = LocalArtifactStore::new(tmp.path()).unwrap();
        let lease_dir = store.in_flight_lease_dir();

        // Worker 1 (Web replica 1 handling streaming upload)
        let worker1_reg = InFlightUploadRegistry::with_shared_dir(lease_dir.clone());

        // Worker 2 (Standalone background GC process with separate in-memory registry)
        let worker2_gc_reg = InFlightUploadRegistry::with_shared_dir(lease_dir);

        let lease_id = "upload_cross_worker_005";
        worker1_reg.register_lease(lease_id, 60);

        let chunk_bytes = b"shared durable chunk across worker processes";
        let chunk_hash = blake3::hash(chunk_bytes).to_hex().to_string();
        store.store_chunk(&chunk_hash, chunk_bytes).unwrap();
        worker1_reg.pin_chunk(lease_id, &chunk_hash);

        let all_chunks = vec![chunk_hash.clone()];

        // Worker 2 runs GC with 0s grace period.
        // Worker 2's registry has an EMPTY in-memory map, but reads the shared lease directory!
        let rep1 = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[], // unmanifested
            &all_chunks,
            Some(&worker2_gc_reg),
            0, // 0 grace
            false,
        )
        .unwrap();

        assert_eq!(
            rep1.in_flight_protected_chunks, 1,
            "Worker 2 must protect chunk pinned by Worker 1 via shared lease"
        );
        assert_eq!(rep1.candidate_chunks, 0);
        assert_eq!(rep1.reclaimed_bytes, 0);

        // Heartbeat from Worker 1
        assert!(worker1_reg.heartbeat(lease_id, 120));

        // Worker 1 completes the upload and commits manifest
        let manifest = ArtifactManifest::new(
            "art_shared".into(),
            chunk_hash.clone(),
            chunk_bytes.len() as u64,
            chunk_bytes.len() as u32,
            vec![ChunkEntry {
                index: 0,
                offset: 0,
                length: chunk_bytes.len() as u32,
                chunk_hash: chunk_hash.clone(),
            }],
            None,
        )
        .unwrap();
        let m_id = store.store_manifest(&manifest).unwrap();
        worker1_reg.complete_lease(lease_id);

        // Worker 2 runs GC again: lease is gone from disk, but manifest now makes chunk reachable!
        let rep2 = ArtifactGcCollector::collect_garbage_with_registry(
            &store,
            &[m_id],
            &all_chunks,
            Some(&worker2_gc_reg),
            0,
            false,
        )
        .unwrap();
        assert_eq!(rep2.reachable_chunks, 1);
        assert_eq!(rep2.candidate_chunks, 0);
        assert_eq!(rep2.in_flight_protected_chunks, 0);
    }
}
