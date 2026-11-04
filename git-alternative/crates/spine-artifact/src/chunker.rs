use crate::manifest::{ArtifactManifest, ChunkEntry, DEFAULT_CHUNK_SIZE};
use std::io::{Read, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ChunkerError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Corrupted chunk {index}: expected hash {expected}, got {computed}")]
    CorruptedChunk {
        index: u32,
        expected: String,
        computed: String,
    },
    #[error("Corrupted whole object: expected hash {expected}, got {computed}")]
    CorruptedObject { expected: String, computed: String },
    #[error("Manifest error: {0}")]
    Manifest(#[from] crate::manifest::ManifestError),
    #[error("Missing chunk {0}")]
    MissingChunk(u32),
}

/// Slices a stream or reader into bounded chunks, calculating per-chunk and whole-object digests.
pub struct StreamChunker {
    chunk_size: u32,
}

pub struct ChunkData {
    pub entry: ChunkEntry,
    pub bytes: Vec<u8>,
}

pub struct ChunkingResult {
    pub manifest: ArtifactManifest,
    pub chunks: Vec<ChunkData>,
}

impl StreamChunker {
    pub fn new(chunk_size: Option<u32>) -> Self {
        Self {
            chunk_size: chunk_size.unwrap_or(DEFAULT_CHUNK_SIZE),
        }
    }

    /// Reads from `reader` in bounded memory and slices into chunks.
    pub fn chunk_reader<R: Read>(
        &self,
        mut reader: R,
        artifact_id: &str,
        content_type: Option<String>,
    ) -> Result<ChunkingResult, ChunkerError> {
        let mut whole_hasher = blake3::Hasher::new();
        let mut chunks = Vec::new();
        let mut chunk_entries = Vec::new();
        let mut offset = 0u64;
        let mut index = 0u32;
        let mut total_size = 0u64;

        let mut buffer = vec![0u8; self.chunk_size as usize];

        loop {
            let mut chunk_bytes = Vec::with_capacity(self.chunk_size as usize);
            let mut bytes_read_for_chunk = 0usize;

            while bytes_read_for_chunk < self.chunk_size as usize {
                let to_read = (self.chunk_size as usize) - bytes_read_for_chunk;
                let n = reader.read(&mut buffer[..to_read])?;
                if n == 0 {
                    break;
                }
                chunk_bytes.extend_from_slice(&buffer[..n]);
                bytes_read_for_chunk += n;
            }

            if chunk_bytes.is_empty() {
                // If total_size == 0, we still emit one 0-byte chunk or handle empty file
                if index == 0 {
                    let chunk_hash = blake3::hash(&[]).to_hex().to_string();
                    let entry = ChunkEntry {
                        index: 0,
                        offset: 0,
                        length: 0,
                        chunk_hash,
                    };
                    chunk_entries.push(entry.clone());
                    chunks.push(ChunkData {
                        entry,
                        bytes: Vec::new(),
                    });
                }
                break;
            }

            whole_hasher.update(&chunk_bytes);
            let chunk_hash = blake3::hash(&chunk_bytes).to_hex().to_string();
            let len = chunk_bytes.len() as u32;

            let entry = ChunkEntry {
                index,
                offset,
                length: len,
                chunk_hash,
            };

            offset += len as u64;
            total_size += len as u64;
            index += 1;

            chunk_entries.push(entry.clone());
            chunks.push(ChunkData {
                entry,
                bytes: chunk_bytes,
            });

            if bytes_read_for_chunk < self.chunk_size as usize {
                // Reached EOF
                break;
            }
        }

        let whole_object_hash = whole_hasher.finalize().to_hex().to_string();

        let manifest = ArtifactManifest::new(
            artifact_id.to_string(),
            whole_object_hash,
            total_size,
            self.chunk_size,
            chunk_entries,
            content_type,
        )?;

        Ok(ChunkingResult { manifest, chunks })
    }
}

/// Reconstructs a stream of an artifact from verified individual chunks.
pub struct ObjectReconstructor;

impl ObjectReconstructor {
    /// Writes chunks sequentially into `writer`, verifying each chunk hash and the overall digest.
    pub fn reconstruct<W: Write, F>(
        manifest: &ArtifactManifest,
        mut get_chunk_fn: F,
        mut writer: W,
    ) -> Result<u64, ChunkerError>
    where
        F: FnMut(u32) -> Result<Vec<u8>, ChunkerError>,
    {
        let mut whole_hasher = blake3::Hasher::new();
        let mut total_written = 0u64;

        for entry in &manifest.chunks {
            let chunk_bytes = get_chunk_fn(entry.index)?;

            if chunk_bytes.len() as u32 != entry.length {
                return Err(ChunkerError::CorruptedChunk {
                    index: entry.index,
                    expected: format!("{} bytes", entry.length),
                    computed: format!("{} bytes", chunk_bytes.len()),
                });
            }

            let computed_chunk_hash = blake3::hash(&chunk_bytes).to_hex().to_string();
            if computed_chunk_hash != entry.chunk_hash {
                return Err(ChunkerError::CorruptedChunk {
                    index: entry.index,
                    expected: entry.chunk_hash.clone(),
                    computed: computed_chunk_hash,
                });
            }

            whole_hasher.update(&chunk_bytes);
            writer.write_all(&chunk_bytes)?;
            total_written += chunk_bytes.len() as u64;
        }

        writer.flush()?;

        let computed_whole_hash = whole_hasher.finalize().to_hex().to_string();
        if computed_whole_hash != manifest.whole_object_hash {
            return Err(ChunkerError::CorruptedObject {
                expected: manifest.whole_object_hash.clone(),
                computed: computed_whole_hash,
            });
        }

        Ok(total_written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chunking_and_reconstruction() {
        // Create 250 KB data
        let mut sample_data = Vec::with_capacity(250 * 1024);
        for i in 0..sample_data.capacity() {
            sample_data.push((i % 256) as u8);
        }

        // Chunk with 64 KB chunk size (yields 4 chunks: 64, 64, 64, 58 KB)
        let chunker = StreamChunker::new(Some(64 * 1024));
        let res = chunker
            .chunk_reader(&sample_data[..], "art_test_1", None)
            .expect("Chunking should succeed");

        assert_eq!(res.manifest.chunk_count, 4);
        assert_eq!(res.manifest.total_size, 250 * 1024);
        assert_eq!(res.chunks.len(), 4);

        // Reconstruct
        let mut reconstructed = Vec::new();
        let total_written = ObjectReconstructor::reconstruct(
            &res.manifest,
            |idx| Ok(res.chunks[idx as usize].bytes.clone()),
            &mut reconstructed,
        )
        .expect("Reconstruction should succeed");

        assert_eq!(total_written, 250 * 1024);
        assert_eq!(reconstructed, sample_data);
    }

    #[test]
    fn test_corrupted_chunk_detection() {
        let sample_data = b"Hello world large file testing content";
        let chunker = StreamChunker::new(Some(10));
        let res = chunker
            .chunk_reader(&sample_data[..], "art_corrupt", None)
            .unwrap();

        let mut reconstructed = Vec::new();
        // Tamper with chunk 1
        let err = ObjectReconstructor::reconstruct(
            &res.manifest,
            |idx| {
                let mut b = res.chunks[idx as usize].bytes.clone();
                if idx == 1 {
                    b[0] ^= 0xFF; // flip bit
                }
                Ok(b)
            },
            &mut reconstructed,
        );

        assert!(matches!(err, Err(ChunkerError::CorruptedChunk { .. })));
    }
}
