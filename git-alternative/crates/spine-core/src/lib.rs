//! # spine-core
//!
//! Core primitives, object identifiers, object types, and domain-separated hashing
//! for the Spine Version Control System.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::Digest;
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

/// The canonical domain separator prefix for all Spine content-addressed hashing.
pub const DOMAIN_PREFIX: &[u8] = b"SPINE\0";

#[derive(Error, Debug, PartialEq, Eq)]
pub enum CoreError {
    #[error("Invalid hexadecimal hash string: {0}")]
    InvalidHex(String),
    #[error("Invalid object type byte: {0:#04x}")]
    InvalidObjectType(u8),
    #[error("Invalid file mode integer: {0:#08o}")]
    InvalidFileMode(u32),
    #[error("Invalid hash algorithm identifier: {0}")]
    InvalidHashAlgorithm(u8),
}

/// Cryptographic hash algorithm used for content-addressed identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[repr(u8)]
pub enum HashAlgorithm {
    #[default]
    Blake3 = 1,
    Sha256 = 2,
}

impl HashAlgorithm {
    pub fn from_u8(val: u8) -> Result<Self, CoreError> {
        match val {
            1 => Ok(Self::Blake3),
            2 => Ok(Self::Sha256),
            other => Err(CoreError::InvalidHashAlgorithm(other)),
        }
    }
}

/// Cryptographic 32-byte object identifier.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    pub const ZERO: Self = Self([0u8; 32]);

    pub const fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(64);
        for byte in &self.0 {
            use std::fmt::Write;
            write!(&mut s, "{:02x}", byte).unwrap();
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self, CoreError> {
        if s.len() != 64 {
            return Err(CoreError::InvalidHex(format!(
                "Expected 64 characters, got {}",
                s.len()
            )));
        }
        let mut bytes = [0u8; 32];
        for i in 0..32 {
            let chunk = &s[i * 2..i * 2 + 2];
            bytes[i] =
                u8::from_str_radix(chunk, 16).map_err(|_| CoreError::InvalidHex(s.to_string()))?;
        }
        Ok(Self(bytes))
    }

    /// Computes an `ObjectId` over the payload using the specified algorithm and domain separation:
    /// `SPINE\0<type_byte>\0<len_u64_le>\0<payload>`
    pub fn compute(algo: HashAlgorithm, object_type: ObjectType, payload: &[u8]) -> Self {
        let mut header = Vec::with_capacity(DOMAIN_PREFIX.len() + 1 + 1 + 8 + 1);
        header.extend_from_slice(DOMAIN_PREFIX);
        header.push(object_type.as_u8());
        header.push(0x00);
        header.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        header.push(0x00);

        match algo {
            HashAlgorithm::Blake3 => {
                let mut hasher = blake3::Hasher::new();
                hasher.update(&header);
                hasher.update(payload);
                let hash = hasher.finalize();
                Self(*hash.as_bytes())
            }
            HashAlgorithm::Sha256 => {
                let mut hasher = sha2::Sha256::new();
                hasher.update(&header);
                hasher.update(payload);
                let result = hasher.finalize();
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&result);
                Self(bytes)
            }
        }
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({})", self.to_hex())
    }
}

impl FromStr for ObjectId {
    type Err = CoreError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl Serialize for ObjectId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for ObjectId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}

/// Fundamental object types supported by Spine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u8)]
pub enum ObjectType {
    Blob = 0x01,
    Tree = 0x02,
    Commit = 0x03,
    Tag = 0x04,
}

impl ObjectType {
    pub const fn as_u8(&self) -> u8 {
        *self as u8
    }

    pub fn from_u8(val: u8) -> Result<Self, CoreError> {
        match val {
            0x01 => Ok(Self::Blob),
            0x02 => Ok(Self::Tree),
            0x03 => Ok(Self::Commit),
            0x04 => Ok(Self::Tag),
            other => Err(CoreError::InvalidObjectType(other)),
        }
    }
}

/// Standard file mode flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(u32)]
pub enum FileMode {
    Regular = 0o100644,
    Executable = 0o100755,
    Symlink = 0o120000,
    Directory = 0o040000,
}

impl FileMode {
    pub const fn as_u32(&self) -> u32 {
        *self as u32
    }

    pub fn from_u32(val: u32) -> Result<Self, CoreError> {
        match val {
            0o100644 => Ok(Self::Regular),
            0o100755 => Ok(Self::Executable),
            0o120000 => Ok(Self::Symlink),
            0o040000 => Ok(Self::Directory),
            other => Err(CoreError::InvalidFileMode(other)),
        }
    }
}

/// Single entry in a Tree object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TreeEntry {
    pub mode: FileMode,
    pub name: String,
    pub object_id: ObjectId,
}

/// Immutable commit snapshot object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Commit {
    pub root_tree: ObjectId,
    pub parents: Vec<ObjectId>,
    pub author_name: String,
    pub author_email: String,
    pub timestamp_nanos: i64,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_object_id_hex_roundtrip() {
        let original = [0xabu8; 32];
        let oid = ObjectId::from_bytes(original);
        let hex = oid.to_hex();
        assert_eq!(hex.len(), 64);
        let recovered = ObjectId::from_hex(&hex).unwrap();
        assert_eq!(oid, recovered);
    }

    #[test]
    fn test_domain_separation_prevents_collision() {
        let payload = b"common byte sequence";
        let blob_id = ObjectId::compute(HashAlgorithm::Blake3, ObjectType::Blob, payload);
        let tree_id = ObjectId::compute(HashAlgorithm::Blake3, ObjectType::Tree, payload);
        let commit_id = ObjectId::compute(HashAlgorithm::Blake3, ObjectType::Commit, payload);

        assert_ne!(blob_id, tree_id);
        assert_ne!(tree_id, commit_id);
        assert_ne!(blob_id, commit_id);
    }

    #[test]
    fn test_hash_algorithm_blake3_vs_sha256() {
        let payload = b"hello spine";
        let blake3_id = ObjectId::compute(HashAlgorithm::Blake3, ObjectType::Blob, payload);
        let sha256_id = ObjectId::compute(HashAlgorithm::Sha256, ObjectType::Blob, payload);

        assert_ne!(blake3_id, sha256_id);
    }
}
