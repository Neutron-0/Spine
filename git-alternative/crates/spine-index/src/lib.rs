//! # spine-index
//!
//! Binary staging manifest and fast filesystem stat cache for Spine repositories.

use blake3::Hasher;
use spine_core::{FileMode, ObjectId};
use std::collections::BTreeMap;
use std::convert::TryInto;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::Path;
use thiserror::Error;
use uuid::Uuid;

pub const INDEX_MAGIC: &[u8; 8] = b"SPININDX";
pub const INDEX_VERSION: u16 = 1;

#[derive(Error, Debug)]
pub enum IndexError {
    #[error("Buffer too short: expected {expected}, found {actual}")]
    BufferTooShort { expected: usize, actual: usize },
    #[error("Invalid index magic identifier")]
    InvalidMagic,
    #[error("Unsupported index version: {0}")]
    UnsupportedVersion(u16),
    #[error("Index checksum mismatch: file corrupted")]
    ChecksumMismatch,
    #[error("Invalid UTF-8 in index path: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    #[error("Core error: {0}")]
    Core(#[from] spine_core::CoreError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum StageStatus {
    Clean = 0,
    Staged = 1,
    Deleted = 2,
}

impl StageStatus {
    pub fn from_u8(val: u8) -> Self {
        match val {
            1 => Self::Staged,
            2 => Self::Deleted,
            _ => Self::Clean,
        }
    }
}

/// A cached file record in the staging index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub path: String,
    pub mode: FileMode,
    pub file_size: u64,
    pub mtime_secs: i64,
    pub mtime_nanos: u32,
    pub inode: u64,
    pub object_id: ObjectId,
    pub stage_status: StageStatus,
}

/// In-memory representation of the binary index (`.spine/index`).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Index {
    entries: BTreeMap<String, IndexEntry>,
}

impl Index {
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    pub fn add_or_update(&mut self, entry: IndexEntry) {
        self.entries.insert(entry.path.clone(), entry);
    }

    pub fn remove(&mut self, path: &str) -> Option<IndexEntry> {
        self.entries.remove(path)
    }

    pub fn get(&self, path: &str) -> Option<&IndexEntry> {
        self.entries.get(path)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = &IndexEntry> {
        self.entries.values()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Serializes index entries into canonical binary format with a trailing BLAKE3 checksum.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(INDEX_MAGIC);
        buf.extend_from_slice(&INDEX_VERSION.to_le_bytes());
        buf.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());

        for entry in self.entries.values() {
            let path_bytes = entry.path.as_bytes();
            buf.extend_from_slice(&(path_bytes.len() as u16).to_le_bytes());
            buf.extend_from_slice(path_bytes);
            buf.extend_from_slice(&entry.mode.as_u32().to_le_bytes());
            buf.extend_from_slice(&entry.file_size.to_le_bytes());
            buf.extend_from_slice(&entry.mtime_secs.to_le_bytes());
            buf.extend_from_slice(&entry.mtime_nanos.to_le_bytes());
            buf.extend_from_slice(&entry.inode.to_le_bytes());
            buf.extend_from_slice(entry.object_id.as_bytes());
            buf.push(entry.stage_status as u8);
        }

        // Calculate and append 32-byte BLAKE3 checksum
        let mut hasher = Hasher::new();
        hasher.update(&buf);
        let hash = hasher.finalize();
        buf.extend_from_slice(hash.as_bytes());

        buf
    }

    /// Deserializes a binary index buffer, verifying the trailing cryptographic checksum.
    pub fn deserialize(buf: &[u8]) -> Result<Self, IndexError> {
        if buf.len() < 8 + 2 + 4 + 32 {
            return Err(IndexError::BufferTooShort {
                expected: 46,
                actual: buf.len(),
            });
        }

        let content_len = buf.len() - 32;
        let content_slice = &buf[..content_len];
        let checksum_slice = &buf[content_len..];

        let mut hasher = Hasher::new();
        hasher.update(content_slice);
        let expected_hash = hasher.finalize();
        if expected_hash.as_bytes() != checksum_slice {
            return Err(IndexError::ChecksumMismatch);
        }

        if &content_slice[0..8] != INDEX_MAGIC {
            return Err(IndexError::InvalidMagic);
        }

        let version = u16::from_le_bytes(content_slice[8..10].try_into().unwrap());
        if version != INDEX_VERSION {
            return Err(IndexError::UnsupportedVersion(version));
        }

        let count = u32::from_le_bytes(content_slice[10..14].try_into().unwrap()) as usize;
        let mut cursor = 14;
        let mut entries = BTreeMap::new();

        for _ in 0..count {
            if cursor + 2 > content_len {
                return Err(IndexError::BufferTooShort {
                    expected: cursor + 2,
                    actual: content_len,
                });
            }
            let path_len =
                u16::from_le_bytes(content_slice[cursor..cursor + 2].try_into().unwrap()) as usize;
            cursor += 2;

            if cursor + path_len + 4 + 8 + 8 + 4 + 8 + 32 + 1 > content_len {
                return Err(IndexError::BufferTooShort {
                    expected: cursor + path_len + 65,
                    actual: content_len,
                });
            }

            let path = String::from_utf8(content_slice[cursor..cursor + path_len].to_vec())?;
            cursor += path_len;

            let mode_raw =
                u32::from_le_bytes(content_slice[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;
            let mode = FileMode::from_u32(mode_raw)?;

            let file_size =
                u64::from_le_bytes(content_slice[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;

            let mtime_secs =
                i64::from_le_bytes(content_slice[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;

            let mtime_nanos =
                u32::from_le_bytes(content_slice[cursor..cursor + 4].try_into().unwrap());
            cursor += 4;

            let inode = u64::from_le_bytes(content_slice[cursor..cursor + 8].try_into().unwrap());
            cursor += 8;

            let mut oid_bytes = [0u8; 32];
            oid_bytes.copy_from_slice(&content_slice[cursor..cursor + 32]);
            let object_id = ObjectId::from_bytes(oid_bytes);
            cursor += 32;

            let stage_status = StageStatus::from_u8(content_slice[cursor]);
            cursor += 1;

            entries.insert(
                path.clone(),
                IndexEntry {
                    path,
                    mode,
                    file_size,
                    mtime_secs,
                    mtime_nanos,
                    inode,
                    object_id,
                    stage_status,
                },
            );
        }

        Ok(Self { entries })
    }

    /// Atomically writes the index to disk using a temporary file.
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), IndexError> {
        let dest = path.as_ref();
        let parent = dest.parent().unwrap_or_else(|| Path::new("."));
        let temp_path = parent.join(format!("idx-{}", Uuid::new_v4()));

        let data = self.serialize();
        {
            let mut f = File::create(&temp_path)?;
            f.write_all(&data)?;
            f.sync_all()?;
        }

        if let Err(e) = fs::rename(&temp_path, dest) {
            // Windows fallback if destination is locked or collision occurs
            let _ = fs::remove_file(dest);
            fs::rename(&temp_path, dest).map_err(|_| IndexError::Io(e))?;
        }

        Ok(())
    }

    /// Reads and verifies an index from disk. Returns an empty index if file does not exist.
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, IndexError> {
        let p = path.as_ref();
        if !p.exists() {
            return Ok(Self::new());
        }

        let mut f = File::open(p)?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        Self::deserialize(&buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_index_serialization_and_checksum_roundtrip() {
        let mut index = Index::new();
        index.add_or_update(IndexEntry {
            path: "src/main.rs".into(),
            mode: FileMode::Regular,
            file_size: 1024,
            mtime_secs: 1730000000,
            mtime_nanos: 500,
            inode: 123456,
            object_id: ObjectId::from_bytes([0x42; 32]),
            stage_status: StageStatus::Staged,
        });

        let serialized = index.serialize();
        let loaded = Index::deserialize(&serialized).unwrap();
        assert_eq!(index, loaded);
    }

    #[test]
    fn test_corrupt_index_checksum_fails() {
        let mut index = Index::new();
        index.add_or_update(IndexEntry {
            path: "test.txt".into(),
            mode: FileMode::Regular,
            file_size: 10,
            mtime_secs: 100,
            mtime_nanos: 0,
            inode: 1,
            object_id: ObjectId::from_bytes([0x11; 32]),
            stage_status: StageStatus::Clean,
        });

        let mut data = index.serialize();
        // Tamper with content byte
        data[15] ^= 0xff;
        assert!(matches!(
            Index::deserialize(&data),
            Err(IndexError::ChecksumMismatch)
        ));
    }
}
