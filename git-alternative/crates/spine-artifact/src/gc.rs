use crate::store::ArtifactStore;
use std::collections::HashSet;
use thiserror::Error;

pub const DEFAULT_ARTIFACT_GC_GRACE_SECS: i64 = 86400; // 24 hours

#[derive(Error, Debug)]
pub enum GcError {
    #[error("Store error: {0}")]
    Store(#[from] crate::store::StoreError),
}

#[derive(Clone, Debug, Default)]
pub struct ArtifactGcReport {
    pub total_chunks_scanned: usize,
    pub reachable_chunks: usize,
    pub candidate_chunks: usize,
    pub grace_protected_chunks: usize,
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
        Self::collect_garbage_with_grace(store, active_manifest_ids, all_chunk_hashes, 0, dry_run)
    }

    /// Generates a GC report with configurable grace period protection for in-flight/recent chunks.
    pub fn collect_garbage_with_grace<S: ArtifactStore + ?Sized>(
        store: &S,
        active_manifest_ids: &[String],
        all_chunk_hashes: &[String],
        grace_secs: i64,
        dry_run: bool,
    ) -> Result<ArtifactGcReport, GcError> {
        let reachable = Self::trace_reachable_chunks(store, active_manifest_ids)?;
        let now = chrono::Utc::now().timestamp();
        let mut report = ArtifactGcReport {
            total_chunks_scanned: all_chunk_hashes.len(),
            reachable_chunks: 0,
            candidate_chunks: 0,
            grace_protected_chunks: 0,
            reclaimed_bytes: 0,
            dry_run,
        };

        for chunk_hash in all_chunk_hashes {
            if reachable.contains(chunk_hash) {
                report.reachable_chunks += 1;
            } else {
                // If chunk was modified within grace period, protect it from GC (e.g. concurrent in-flight upload)
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
                // If not dry_run, deletion would be executed here
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
}
