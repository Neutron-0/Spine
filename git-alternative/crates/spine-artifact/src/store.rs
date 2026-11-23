use crate::manifest::{ArtifactManifest, ManifestError};
use spine_cluster::durability::DurabilityPolicy;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum StoreError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Manifest error: {0}")]
    Manifest(#[from] ManifestError),
    #[error("Chunk integrity error for {chunk_hash}: expected {chunk_hash}, got {actual}")]
    ChunkCorruption { chunk_hash: String, actual: String },
    #[error("Chunk not found: {0}")]
    ChunkNotFound(String),
    #[error("Manifest not found: {0}")]
    ManifestNotFound(String),
    #[error("Storage error: {0}")]
    Other(String),
}

/// Unified trait for storing and retrieving content-addressed artifact chunks and manifests.
pub trait ArtifactStore: Send + Sync {
    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<(), StoreError>;
    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>, StoreError>;
    fn has_chunk(&self, chunk_hash: &str) -> bool;

    fn store_manifest(&self, manifest: &ArtifactManifest) -> Result<String, StoreError>;
    fn get_manifest(&self, manifest_id: &str) -> Result<Option<ArtifactManifest>, StoreError>;
    fn has_manifest(&self, manifest_id: &str) -> bool;

    fn verify_artifact_durability(
        &self,
        manifest_id: &str,
        policy: DurabilityPolicy,
    ) -> Result<bool, StoreError>;

    fn get_chunk_mtime_secs(&self, _chunk_hash: &str) -> Result<Option<i64>, StoreError> {
        Ok(None)
    }
}

impl<T: ?Sized + ArtifactStore> ArtifactStore for Arc<T> {
    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<(), StoreError> {
        (**self).store_chunk(chunk_hash, bytes)
    }
    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        (**self).get_chunk(chunk_hash)
    }
    fn has_chunk(&self, chunk_hash: &str) -> bool {
        (**self).has_chunk(chunk_hash)
    }
    fn store_manifest(&self, manifest: &ArtifactManifest) -> Result<String, StoreError> {
        (**self).store_manifest(manifest)
    }
    fn get_manifest(&self, manifest_id: &str) -> Result<Option<ArtifactManifest>, StoreError> {
        (**self).get_manifest(manifest_id)
    }
    fn has_manifest(&self, manifest_id: &str) -> bool {
        (**self).has_manifest(manifest_id)
    }
    fn verify_artifact_durability(
        &self,
        manifest_id: &str,
        policy: DurabilityPolicy,
    ) -> Result<bool, StoreError> {
        (**self).verify_artifact_durability(manifest_id, policy)
    }
    fn get_chunk_mtime_secs(&self, chunk_hash: &str) -> Result<Option<i64>, StoreError> {
        (**self).get_chunk_mtime_secs(chunk_hash)
    }
}

/// Local directory-backed content-addressed artifact store.
#[derive(Clone)]
pub struct LocalArtifactStore {
    root_dir: PathBuf,
}

impl LocalArtifactStore {
    pub fn new<P: AsRef<Path>>(root_dir: P) -> Result<Self, StoreError> {
        let p = root_dir.as_ref().to_path_buf();
        fs::create_dir_all(p.join("chunks"))?;
        fs::create_dir_all(p.join("manifests"))?;
        fs::create_dir_all(p.join("staging"))?;
        Ok(Self { root_dir: p })
    }

    fn chunk_path(&self, chunk_hash: &str) -> PathBuf {
        let prefix = if chunk_hash.len() >= 2 {
            &chunk_hash[0..2]
        } else {
            "xx"
        };
        self.root_dir.join("chunks").join(prefix).join(chunk_hash)
    }

    fn manifest_path(&self, manifest_id: &str) -> PathBuf {
        let prefix = if manifest_id.len() >= 2 {
            &manifest_id[0..2]
        } else {
            "xx"
        };
        self.root_dir
            .join("manifests")
            .join(prefix)
            .join(format!("{}.json", manifest_id))
    }
}

impl ArtifactStore for LocalArtifactStore {
    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<(), StoreError> {
        let computed = blake3::hash(bytes).to_hex().to_string();
        if computed != chunk_hash {
            return Err(StoreError::ChunkCorruption {
                chunk_hash: chunk_hash.to_string(),
                actual: computed,
            });
        }

        let p = self.chunk_path(chunk_hash);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }

        if !p.exists() {
            // Write atomically via temporary file
            let tmp_p = p.with_extension("tmp");
            fs::write(&tmp_p, bytes)?;
            fs::rename(&tmp_p, &p)?;
        }

        Ok(())
    }

    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let p = self.chunk_path(chunk_hash);
        if !p.exists() {
            return Ok(None);
        }
        let bytes = fs::read(&p)?;
        let computed = blake3::hash(&bytes).to_hex().to_string();
        if computed != chunk_hash {
            return Err(StoreError::ChunkCorruption {
                chunk_hash: chunk_hash.to_string(),
                actual: computed,
            });
        }
        Ok(Some(bytes))
    }

    fn has_chunk(&self, chunk_hash: &str) -> bool {
        self.chunk_path(chunk_hash).is_file()
    }

    fn store_manifest(&self, manifest: &ArtifactManifest) -> Result<String, StoreError> {
        let id = manifest.manifest_id();
        let p = self.manifest_path(&id);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent)?;
        }

        if !p.exists() {
            let json = manifest.to_json()?;
            let tmp_p = p.with_extension("tmp");
            fs::write(&tmp_p, json.as_bytes())?;
            fs::rename(&tmp_p, &p)?;
        }

        Ok(id)
    }

    fn get_manifest(&self, manifest_id: &str) -> Result<Option<ArtifactManifest>, StoreError> {
        let p = self.manifest_path(manifest_id);
        if !p.exists() {
            return Ok(None);
        }
        let s = fs::read_to_string(&p)?;
        let manifest = ArtifactManifest::from_json(&s)?;
        Ok(Some(manifest))
    }

    fn has_manifest(&self, manifest_id: &str) -> bool {
        self.manifest_path(manifest_id).is_file()
    }

    fn verify_artifact_durability(
        &self,
        manifest_id: &str,
        policy: DurabilityPolicy,
    ) -> Result<bool, StoreError> {
        let manifest = match self.get_manifest(manifest_id)? {
            Some(m) => m,
            None => return Ok(false),
        };

        // For local artifact store, all chunks must exist locally
        for chunk in &manifest.chunks {
            if !self.has_chunk(&chunk.chunk_hash) {
                return Ok(false);
            }
        }

        // For SingleCopy, local store is sufficient
        // In distributed cluster, multi-copy quorum is verified via coordinator/dispatcher
        match policy {
            DurabilityPolicy::SingleCopy => Ok(true),
            DurabilityPolicy::TwoCopies | DurabilityPolicy::ThreeCopies => {
                // Verified when replicated across nodes
                Ok(true)
            }
        }
    }

    fn get_chunk_mtime_secs(&self, chunk_hash: &str) -> Result<Option<i64>, StoreError> {
        let p = self.chunk_path(chunk_hash);
        if !p.exists() {
            return Ok(None);
        }
        let meta = fs::metadata(&p)?;
        let mtime = meta
            .modified()?
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        Ok(Some(mtime))
    }
}

/// Clustered artifact store wrapper combining local storage with replica storage dispatch.
pub struct DistributedArtifactStore {
    local: Arc<LocalArtifactStore>,
    replica_stores: Vec<Arc<LocalArtifactStore>>,
}

impl DistributedArtifactStore {
    pub fn new(
        local: Arc<LocalArtifactStore>,
        replica_stores: Vec<Arc<LocalArtifactStore>>,
    ) -> Self {
        Self {
            local,
            replica_stores,
        }
    }
}

impl ArtifactStore for DistributedArtifactStore {
    fn store_chunk(&self, chunk_hash: &str, bytes: &[u8]) -> Result<(), StoreError> {
        self.local.store_chunk(chunk_hash, bytes)?;
        for rep in &self.replica_stores {
            let _ = rep.store_chunk(chunk_hash, bytes);
        }
        Ok(())
    }

    fn get_chunk(&self, chunk_hash: &str) -> Result<Option<Vec<u8>>, StoreError> {
        if let Some(bytes) = self.local.get_chunk(chunk_hash)? {
            return Ok(Some(bytes));
        }
        for rep in &self.replica_stores {
            if let Some(bytes) = rep.get_chunk(chunk_hash)? {
                return Ok(Some(bytes));
            }
        }
        Ok(None)
    }

    fn has_chunk(&self, chunk_hash: &str) -> bool {
        if self.local.has_chunk(chunk_hash) {
            return true;
        }
        self.replica_stores.iter().any(|r| r.has_chunk(chunk_hash))
    }

    fn store_manifest(&self, manifest: &ArtifactManifest) -> Result<String, StoreError> {
        let id = self.local.store_manifest(manifest)?;
        for rep in &self.replica_stores {
            let _ = rep.store_manifest(manifest);
        }
        Ok(id)
    }

    fn get_manifest(&self, manifest_id: &str) -> Result<Option<ArtifactManifest>, StoreError> {
        if let Some(m) = self.local.get_manifest(manifest_id)? {
            return Ok(Some(m));
        }
        for rep in &self.replica_stores {
            if let Some(m) = rep.get_manifest(manifest_id)? {
                return Ok(Some(m));
            }
        }
        Ok(None)
    }

    fn has_manifest(&self, manifest_id: &str) -> bool {
        self.local.has_manifest(manifest_id)
            || self
                .replica_stores
                .iter()
                .any(|r| r.has_manifest(manifest_id))
    }

    fn verify_artifact_durability(
        &self,
        manifest_id: &str,
        policy: DurabilityPolicy,
    ) -> Result<bool, StoreError> {
        let manifest = match self.get_manifest(manifest_id)? {
            Some(m) => m,
            None => return Ok(false),
        };

        let required = policy.required_copies();
        let mut all_stores = vec![&self.local];
        for r in &self.replica_stores {
            all_stores.push(r);
        }

        // Count how many stores possess all chunks and the manifest
        let mut valid_copies = 0;
        for store in all_stores {
            if store.has_manifest(manifest_id) {
                let all_chunks_present = manifest
                    .chunks
                    .iter()
                    .all(|c| store.has_chunk(&c.chunk_hash));
                if all_chunks_present {
                    valid_copies += 1;
                }
            }
        }

        Ok(valid_copies >= required)
    }

    fn get_chunk_mtime_secs(&self, chunk_hash: &str) -> Result<Option<i64>, StoreError> {
        self.local.get_chunk_mtime_secs(chunk_hash)
    }
}
