use serde::{Deserialize, Serialize};
use thiserror::Error;

pub const DEFAULT_CHUNK_SIZE: u32 = 4 * 1024 * 1024; // 4 MiB
pub const MANIFEST_VERSION: u32 = 1;

#[derive(Error, Debug)]
pub enum ManifestError {
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Validation error: {0}")]
    Validation(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Metadata describing an individual content chunk of an artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChunkEntry {
    pub index: u32,
    pub offset: u64,
    pub length: u32,
    pub chunk_hash: String,
}

/// Immutable, versioned manifest describing a complete reconstructed artifact.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactManifest {
    pub version: u32,
    pub artifact_id: String,
    pub whole_object_hash: String,
    pub total_size: u64,
    pub chunk_size: u32,
    pub chunk_count: u32,
    pub chunks: Vec<ChunkEntry>,
    pub content_type: Option<String>,
    pub created_at_nanos: i64,
    pub finalized: bool,
}

impl ArtifactManifest {
    pub fn new(
        artifact_id: String,
        whole_object_hash: String,
        total_size: u64,
        chunk_size: u32,
        chunks: Vec<ChunkEntry>,
        content_type: Option<String>,
    ) -> Result<Self, ManifestError> {
        let chunk_count = chunks.len() as u32;
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let manifest = Self {
            version: MANIFEST_VERSION,
            artifact_id,
            whole_object_hash,
            total_size,
            chunk_size,
            chunk_count,
            chunks,
            content_type,
            created_at_nanos: now_nanos,
            finalized: false,
        };

        manifest.validate()?;
        Ok(manifest)
    }

    /// Computes the deterministic content-addressed manifest ID (BLAKE3 hash).
    pub fn manifest_id(&self) -> String {
        let json_bytes = serde_json::to_vec(self).unwrap_or_default();
        blake3::hash(&json_bytes).to_hex().to_string()
    }

    /// Validates internal consistency of chunk offsets, lengths, and total size.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.version != MANIFEST_VERSION {
            return Err(ManifestError::Validation(format!(
                "Unsupported manifest version: {}",
                self.version
            )));
        }

        if self.chunks.len() != self.chunk_count as usize {
            return Err(ManifestError::Validation(format!(
                "Chunk count mismatch: expected {}, found {}",
                self.chunk_count,
                self.chunks.len()
            )));
        }

        let mut expected_offset = 0u64;
        for (i, chunk) in self.chunks.iter().enumerate() {
            if chunk.index != i as u32 {
                return Err(ManifestError::Validation(format!(
                    "Out of order chunk at index {}: expected index {}",
                    i, chunk.index
                )));
            }

            if chunk.offset != expected_offset {
                return Err(ManifestError::Validation(format!(
                    "Offset mismatch for chunk {}: expected {}, found {}",
                    i, expected_offset, chunk.offset
                )));
            }

            if chunk.length == 0 && self.total_size > 0 {
                return Err(ManifestError::Validation(format!(
                    "Zero-length chunk at index {}",
                    i
                )));
            }

            if i < self.chunks.len() - 1 && chunk.length != self.chunk_size {
                return Err(ManifestError::Validation(format!(
                    "Non-final chunk {} size {} differs from nominal chunk size {}",
                    i, chunk.length, self.chunk_size
                )));
            }

            expected_offset += chunk.length as u64;
        }

        if expected_offset != self.total_size {
            return Err(ManifestError::Validation(format!(
                "Total size mismatch: sum of chunks is {}, manifest specifies {}",
                expected_offset, self.total_size
            )));
        }

        Ok(())
    }

    pub fn to_json(&self) -> Result<String, ManifestError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    pub fn from_json(s: &str) -> Result<Self, ManifestError> {
        let manifest: Self = serde_json::from_str(s)?;
        manifest.validate()?;
        Ok(manifest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_creation_and_validation() {
        let chunk1 = ChunkEntry {
            index: 0,
            offset: 0,
            length: 100,
            chunk_hash: "hash1".into(),
        };
        let chunk2 = ChunkEntry {
            index: 1,
            offset: 100,
            length: 50,
            chunk_hash: "hash2".into(),
        };

        let manifest = ArtifactManifest::new(
            "art_test".into(),
            "whole_hash".into(),
            150,
            100,
            vec![chunk1, chunk2],
            Some("application/octet-stream".into()),
        )
        .expect("Valid manifest should construct");

        assert_eq!(manifest.chunk_count, 2);
        assert!(!manifest.manifest_id().is_empty());

        let json = manifest.to_json().unwrap();
        let parsed = ArtifactManifest::from_json(&json).unwrap();
        assert_eq!(parsed, manifest);
    }
}
