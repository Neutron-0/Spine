//! # spine-format
//!
//! Deterministic binary serialization and deserialization routines for Spine
//! trees, commits, and container headers.

use spine_core::{Commit, FileMode, ObjectId, ObjectType, TreeEntry};
use std::convert::TryInto;
use thiserror::Error;

pub const LOOSE_OBJECT_MAGIC: &[u8; 8] = b"SPINEOBJ";
pub const LOOSE_OBJECT_VERSION: u16 = 1;
pub const LOOSE_HEADER_SIZE: usize = 16;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum FormatError {
    #[error("Buffer too short: expected at least {expected} bytes, found {actual}")]
    BufferTooShort { expected: usize, actual: usize },
    #[error("Invalid magic identifier: expected {expected:?}, found {actual:?}")]
    InvalidMagic { expected: Vec<u8>, actual: Vec<u8> },
    #[error("Unsupported format version: {0}")]
    UnsupportedVersion(u16),
    #[error("Invalid UTF-8 string encoding: {0}")]
    InvalidUtf8(#[from] std::string::FromUtf8Error),
    #[error("Core error: {0}")]
    Core(#[from] spine_core::CoreError),
    #[error("Corrupt payload: {0}")]
    CorruptPayload(String),
}

/// Loose container header stored at the front of each object file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LooseHeader {
    pub version: u16,
    pub object_type: ObjectType,
}

impl LooseHeader {
    pub fn new(object_type: ObjectType) -> Self {
        Self {
            version: LOOSE_OBJECT_VERSION,
            object_type,
        }
    }

    pub fn serialize(&self) -> [u8; LOOSE_HEADER_SIZE] {
        let mut buf = [0u8; LOOSE_HEADER_SIZE];
        buf[0..8].copy_from_slice(LOOSE_OBJECT_MAGIC);
        buf[8..10].copy_from_slice(&self.version.to_le_bytes());
        buf[10] = self.object_type.as_u8();
        // buf[11..16] are 0x00 reserved
        buf
    }

    pub fn deserialize(buf: &[u8]) -> Result<Self, FormatError> {
        if buf.len() < LOOSE_HEADER_SIZE {
            return Err(FormatError::BufferTooShort {
                expected: LOOSE_HEADER_SIZE,
                actual: buf.len(),
            });
        }
        if &buf[0..8] != LOOSE_OBJECT_MAGIC {
            return Err(FormatError::InvalidMagic {
                expected: LOOSE_OBJECT_MAGIC.to_vec(),
                actual: buf[0..8].to_vec(),
            });
        }
        let version = u16::from_le_bytes(buf[8..10].try_into().unwrap());
        if version != LOOSE_OBJECT_VERSION {
            return Err(FormatError::UnsupportedVersion(version));
        }
        let object_type = ObjectType::from_u8(buf[10])?;
        Ok(Self {
            version,
            object_type,
        })
    }
}

/// Serializes a slice of `TreeEntry` into canonical sorted binary representation.
pub fn serialize_tree(entries: &[TreeEntry]) -> Vec<u8> {
    let mut sorted_entries = entries.to_vec();
    sorted_entries.sort_by(|a, b| a.name.cmp(&b.name));

    let mut buf = Vec::new();
    buf.extend_from_slice(&(sorted_entries.len() as u32).to_le_bytes());

    for entry in sorted_entries {
        buf.extend_from_slice(&entry.mode.as_u32().to_le_bytes());
        let name_bytes = entry.name.as_bytes();
        buf.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
        buf.extend_from_slice(name_bytes);
        buf.extend_from_slice(entry.object_id.as_bytes());
    }
    buf
}

/// Deserializes a binary payload into a list of `TreeEntry`.
pub fn deserialize_tree(payload: &[u8]) -> Result<Vec<TreeEntry>, FormatError> {
    if payload.len() < 4 {
        return Err(FormatError::BufferTooShort {
            expected: 4,
            actual: payload.len(),
        });
    }

    let count = u32::from_le_bytes(payload[0..4].try_into().unwrap()) as usize;
    let mut cursor = 4;
    let mut entries = Vec::with_capacity(count);

    for _ in 0..count {
        if cursor + 4 + 2 > payload.len() {
            return Err(FormatError::BufferTooShort {
                expected: cursor + 6,
                actual: payload.len(),
            });
        }
        let mode_raw = u32::from_le_bytes(payload[cursor..cursor + 4].try_into().unwrap());
        let mode = FileMode::from_u32(mode_raw)?;
        cursor += 4;

        let name_len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
        cursor += 2;

        if cursor + name_len + 32 > payload.len() {
            return Err(FormatError::BufferTooShort {
                expected: cursor + name_len + 32,
                actual: payload.len(),
            });
        }

        let name = String::from_utf8(payload[cursor..cursor + name_len].to_vec())?;
        cursor += name_len;

        let mut oid_bytes = [0u8; 32];
        oid_bytes.copy_from_slice(&payload[cursor..cursor + 32]);
        cursor += 32;

        entries.push(TreeEntry {
            mode,
            name,
            object_id: ObjectId::from_bytes(oid_bytes),
        });
    }

    Ok(entries)
}

/// Serializes a `Commit` into canonical binary representation.
pub fn serialize_commit(commit: &Commit) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(commit.root_tree.as_bytes());

    buf.push(commit.parents.len() as u8);
    for parent in &commit.parents {
        buf.extend_from_slice(parent.as_bytes());
    }

    let author_name_bytes = commit.author_name.as_bytes();
    buf.extend_from_slice(&(author_name_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(author_name_bytes);

    let author_email_bytes = commit.author_email.as_bytes();
    buf.extend_from_slice(&(author_email_bytes.len() as u16).to_le_bytes());
    buf.extend_from_slice(author_email_bytes);

    buf.extend_from_slice(&commit.timestamp_nanos.to_le_bytes());

    let message_bytes = commit.message.as_bytes();
    buf.extend_from_slice(&(message_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(message_bytes);

    buf
}

/// Deserializes a binary payload into a `Commit`.
pub fn deserialize_commit(payload: &[u8]) -> Result<Commit, FormatError> {
    if payload.len() < 32 + 1 {
        return Err(FormatError::BufferTooShort {
            expected: 33,
            actual: payload.len(),
        });
    }

    let mut cursor = 0;
    let mut root_tree_bytes = [0u8; 32];
    root_tree_bytes.copy_from_slice(&payload[cursor..cursor + 32]);
    let root_tree = ObjectId::from_bytes(root_tree_bytes);
    cursor += 32;

    let parent_count = payload[cursor] as usize;
    cursor += 1;

    let mut parents = Vec::with_capacity(parent_count);
    for _ in 0..parent_count {
        if cursor + 32 > payload.len() {
            return Err(FormatError::BufferTooShort {
                expected: cursor + 32,
                actual: payload.len(),
            });
        }
        let mut p_bytes = [0u8; 32];
        p_bytes.copy_from_slice(&payload[cursor..cursor + 32]);
        parents.push(ObjectId::from_bytes(p_bytes));
        cursor += 32;
    }

    if cursor + 2 > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + 2,
            actual: payload.len(),
        });
    }
    let name_len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
    cursor += 2;

    if cursor + name_len > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + name_len,
            actual: payload.len(),
        });
    }
    let author_name = String::from_utf8(payload[cursor..cursor + name_len].to_vec())?;
    cursor += name_len;

    if cursor + 2 > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + 2,
            actual: payload.len(),
        });
    }
    let email_len = u16::from_le_bytes(payload[cursor..cursor + 2].try_into().unwrap()) as usize;
    cursor += 2;

    if cursor + email_len > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + email_len,
            actual: payload.len(),
        });
    }
    let author_email = String::from_utf8(payload[cursor..cursor + email_len].to_vec())?;
    cursor += email_len;

    if cursor + 8 + 4 > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + 12,
            actual: payload.len(),
        });
    }
    let timestamp_nanos = i64::from_le_bytes(payload[cursor..cursor + 8].try_into().unwrap());
    cursor += 8;

    let message_len = u32::from_le_bytes(payload[cursor..cursor + 4].try_into().unwrap()) as usize;
    cursor += 4;

    if cursor + message_len > payload.len() {
        return Err(FormatError::BufferTooShort {
            expected: cursor + message_len,
            actual: payload.len(),
        });
    }
    let message = String::from_utf8(payload[cursor..cursor + message_len].to_vec())?;

    Ok(Commit {
        root_tree,
        parents,
        author_name,
        author_email,
        timestamp_nanos,
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_loose_header_roundtrip() {
        let header = LooseHeader::new(ObjectType::Commit);
        let bytes = header.serialize();
        let decoded = LooseHeader::deserialize(&bytes).unwrap();
        assert_eq!(header, decoded);
    }

    #[test]
    fn test_tree_serialization_roundtrip() {
        let entries = vec![
            TreeEntry {
                mode: FileMode::Regular,
                name: "b.txt".into(),
                object_id: ObjectId::from_bytes([1u8; 32]),
            },
            TreeEntry {
                mode: FileMode::Executable,
                name: "a.sh".into(),
                object_id: ObjectId::from_bytes([2u8; 32]),
            },
        ];
        let bytes = serialize_tree(&entries);
        let decoded = deserialize_tree(&bytes).unwrap();
        // Since serialize_tree sorts by name:
        assert_eq!(decoded.len(), 2);
        assert_eq!(decoded[0].name, "a.sh");
        assert_eq!(decoded[1].name, "b.txt");
    }

    #[test]
    fn test_commit_serialization_roundtrip() {
        let commit = Commit {
            root_tree: ObjectId::from_bytes([3u8; 32]),
            parents: vec![
                ObjectId::from_bytes([4u8; 32]),
                ObjectId::from_bytes([5u8; 32]),
            ],
            author_name: "Developer".into(),
            author_email: "dev@spine.org".into(),
            timestamp_nanos: 1730332800123456789,
            message: "feat: initial commit\n\nwith body".into(),
        };
        let bytes = serialize_commit(&commit);
        let decoded = deserialize_commit(&bytes).unwrap();
        assert_eq!(commit, decoded);
    }
}
