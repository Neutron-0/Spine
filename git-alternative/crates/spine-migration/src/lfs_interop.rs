use sha2::{Digest, Sha256};
use spine_artifact::{ArtifactManifest, ArtifactStore, LocalArtifactStore, StreamChunker};
use std::io::{self, Cursor};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const LFS_POINTER_PREFIX: &str = "version https://git-lfs.github.com/spec/v1\n";

#[derive(Error, Debug)]
pub enum LfsError {
    #[error("I/O error during LFS operation: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid LFS pointer format: {0}")]
    InvalidPointer(String),
    #[error("LFS payload checksum mismatch: expected {expected}, actual {actual}")]
    ChecksumMismatch { expected: String, actual: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitLfsPointer {
    pub oid_sha256: String,
    pub size_bytes: u64,
}

impl GitLfsPointer {
    /// Checks if a blob payload matches the Git LFS pointer format.
    pub fn is_lfs_pointer(payload: &[u8]) -> bool {
        if payload.len() > 1024 {
            // LFS pointers are tiny text files (< 200 bytes)
            return false;
        }
        if let Ok(s) = std::str::from_utf8(payload) {
            s.starts_with(LFS_POINTER_PREFIX) && s.contains("oid sha256:") && s.contains("size ")
        } else {
            false
        }
    }

    /// Parses an LFS pointer from raw bytes.
    pub fn parse(payload: &[u8]) -> Result<Self, LfsError> {
        let s = std::str::from_utf8(payload)
            .map_err(|_| LfsError::InvalidPointer("Invalid UTF-8".to_string()))?;

        if !s.starts_with(LFS_POINTER_PREFIX) {
            return Err(LfsError::InvalidPointer(
                "Missing expected version header".to_string(),
            ));
        }

        let mut oid_sha256 = None;
        let mut size_bytes = None;

        for line in s.lines() {
            let line = line.trim();
            if let Some(rest) = line.strip_prefix("oid sha256:") {
                let trimmed_oid = rest.trim();
                if trimmed_oid.len() == 64 {
                    oid_sha256 = Some(trimmed_oid.to_string());
                }
            } else if let Some(rest) = line.strip_prefix("size ") {
                if let Ok(sz) = rest.trim().parse::<u64>() {
                    size_bytes = Some(sz);
                }
            }
        }

        let oid = oid_sha256
            .ok_or_else(|| LfsError::InvalidPointer("Missing oid sha256 header".to_string()))?;
        let size = size_bytes
            .ok_or_else(|| LfsError::InvalidPointer("Missing size header".to_string()))?;

        Ok(Self {
            oid_sha256: oid,
            size_bytes: size,
        })
    }

    /// Serializes the pointer to standard Git LFS format.
    pub fn serialize(&self) -> Vec<u8> {
        format!(
            "version https://git-lfs.github.com/spec/v1\noid sha256:{}\nsize {}\n",
            self.oid_sha256, self.size_bytes
        )
        .into_bytes()
    }

    /// Computes the Git LFS pointer from raw data.
    pub fn from_data(data: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(data);
        let res = hasher.finalize();
        let hex = hex::encode(res);
        Self {
            oid_sha256: hex,
            size_bytes: data.len() as u64,
        }
    }

    /// Locates the corresponding media file in a `.git/lfs/objects/` store if present.
    pub fn find_local_media_path<P: AsRef<Path>>(&self, git_dir: P) -> Option<PathBuf> {
        if self.oid_sha256.len() < 4 {
            return None;
        }
        let (d1, rest) = self.oid_sha256.split_at(2);
        let (d2, _) = rest.split_at(2);
        let candidate = git_dir
            .as_ref()
            .join("lfs")
            .join("objects")
            .join(d1)
            .join(d2)
            .join(&self.oid_sha256);

        if candidate.exists() && candidate.is_file() {
            Some(candidate)
        } else {
            None
        }
    }

    /// Ingests the actual LFS binary payload into Spine's native artifact storage.
    /// Verifies that SHA-256 and size match this pointer before chunking and storing.
    pub fn import_to_artifact_store<P: AsRef<Path>>(
        &self,
        payload: &[u8],
        store_root: P,
    ) -> Result<ArtifactManifest, LfsError> {
        if payload.len() as u64 != self.size_bytes {
            return Err(LfsError::InvalidPointer(format!(
                "Payload size {} does not match declared size {}",
                payload.len(),
                self.size_bytes
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(payload);
        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != self.oid_sha256 {
            return Err(LfsError::ChecksumMismatch {
                expected: self.oid_sha256.clone(),
                actual: actual_hash,
            });
        }

        let chunker = StreamChunker::new(None);
        let cursor = Cursor::new(payload);
        let result = chunker
            .chunk_reader(
                cursor,
                &self.oid_sha256,
                Some("application/octet-stream".to_string()),
            )
            .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?;

        let store = LocalArtifactStore::new(store_root)
            .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?;

        for chunk in &result.chunks {
            store
                .store_chunk(&chunk.entry.chunk_hash, &chunk.bytes)
                .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?;
        }

        store
            .store_manifest(&result.manifest)
            .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?;

        Ok(result.manifest)
    }

    /// Hydrates the file from native artifact storage back into full binary bytes.
    pub fn hydrate_from_artifact_store<P: AsRef<Path>>(
        &self,
        manifest: &ArtifactManifest,
        store_root: P,
    ) -> Result<Vec<u8>, LfsError> {
        let store = LocalArtifactStore::new(store_root)
            .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?;

        let mut payload = Vec::with_capacity(manifest.total_size as usize);
        for chunk_entry in &manifest.chunks {
            let chunk_bytes = store
                .get_chunk(&chunk_entry.chunk_hash)
                .map_err(|e| LfsError::Io(io::Error::other(e.to_string())))?
                .ok_or_else(|| {
                    LfsError::Io(io::Error::new(
                        io::ErrorKind::NotFound,
                        format!("Missing chunk {}", chunk_entry.chunk_hash),
                    ))
                })?;
            payload.extend_from_slice(&chunk_bytes);
        }

        if payload.len() as u64 != self.size_bytes {
            return Err(LfsError::InvalidPointer(format!(
                "Hydrated size {} does not match pointer size {}",
                payload.len(),
                self.size_bytes
            )));
        }

        let mut hasher = Sha256::new();
        hasher.update(&payload);
        let actual_hash = hex::encode(hasher.finalize());
        if actual_hash != self.oid_sha256 {
            return Err(LfsError::ChecksumMismatch {
                expected: self.oid_sha256.clone(),
                actual: actual_hash,
            });
        }

        Ok(payload)
    }
}
