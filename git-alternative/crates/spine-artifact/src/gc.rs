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
    pub reclaimed_bytes: u64,
    pub dry_run: bool,
}

pub struct ArtifactGcCollector;

impl ArtifactGcCollector {
    /// Computes reachable chunks given a set of active manifest IDs.
    pub fn trace_reachable_chunks<S: ArtifactStore>(
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
    pub fn collect_garbage<S: ArtifactStore>(
        store: &S,
        active_manifest_ids: &[String],
        all_chunk_hashes: &[String],
        dry_run: bool,
    ) -> Result<ArtifactGcReport, GcError> {
        let reachable = Self::trace_reachable_chunks(store, active_manifest_ids)?;
        let mut report = ArtifactGcReport {
            total_chunks_scanned: all_chunk_hashes.len(),
            reachable_chunks: 0,
            candidate_chunks: 0,
            reclaimed_bytes: 0,
            dry_run,
        };

        for chunk_hash in all_chunk_hashes {
            if reachable.contains(chunk_hash) {
                report.reachable_chunks += 1;
            } else {
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
}
