use super::oid::GitOid;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use std::io::{self, Read, Write};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LooseObjectError {
    #[error("I/O error reading/writing loose object: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid loose object header format")]
    InvalidHeader,
    #[error("Corrupt loose object hash: expected {expected}, computed {computed}")]
    HashMismatch { expected: GitOid, computed: GitOid },
    #[error("Invalid object size declared: expected {declared}, read {actual}")]
    SizeMismatch { declared: usize, actual: usize },
    #[error("Unknown Git object type: {0}")]
    UnknownType(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitObjectType {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl GitObjectType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
            Self::Tag => "tag",
        }
    }

    pub fn parse(s: &str) -> Result<Self, LooseObjectError> {
        match s {
            "blob" => Ok(Self::Blob),
            "tree" => Ok(Self::Tree),
            "commit" => Ok(Self::Commit),
            "tag" => Ok(Self::Tag),
            other => Err(LooseObjectError::UnknownType(other.to_string())),
        }
    }
}

/// Reads and decompresses a loose Git object from raw compressed bytes.
pub fn parse_loose_object(
    compressed: &[u8],
    expected_oid: Option<GitOid>,
) -> Result<(GitObjectType, Vec<u8>, GitOid), LooseObjectError> {
    let mut decoder = ZlibDecoder::new(compressed);
    let mut decompressed = Vec::new();
    decoder.read_to_end(&mut decompressed)?;

    // Header is: "<type> <size>\0"
    let null_pos = decompressed
        .iter()
        .position(|&b| b == 0)
        .ok_or(LooseObjectError::InvalidHeader)?;

    let header_str = std::str::from_utf8(&decompressed[..null_pos])
        .map_err(|_| LooseObjectError::InvalidHeader)?;

    let mut parts = header_str.splitn(2, ' ');
    let type_str = parts.next().ok_or(LooseObjectError::InvalidHeader)?;
    let size_str = parts.next().ok_or(LooseObjectError::InvalidHeader)?;

    let obj_type = GitObjectType::parse(type_str)?;
    let declared_size: usize = size_str
        .parse()
        .map_err(|_| LooseObjectError::InvalidHeader)?;

    let payload = decompressed[null_pos + 1..].to_vec();
    if payload.len() != declared_size {
        return Err(LooseObjectError::SizeMismatch {
            declared: declared_size,
            actual: payload.len(),
        });
    }

    let computed_oid = GitOid::compute_object(obj_type.as_str(), &payload);
    if let Some(expected) = expected_oid {
        if computed_oid != expected {
            return Err(LooseObjectError::HashMismatch {
                expected,
                computed: computed_oid,
            });
        }
    }

    Ok((obj_type, payload, computed_oid))
}

/// Serializes and zlib-compresses a Git object into loose format.
pub fn serialize_loose_object(
    obj_type: GitObjectType,
    payload: &[u8],
) -> Result<(Vec<u8>, GitOid), LooseObjectError> {
    let oid = GitOid::compute_object(obj_type.as_str(), payload);
    let header = format!("{} {}\0", obj_type.as_str(), payload.len());

    let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
    encoder.write_all(header.as_bytes())?;
    encoder.write_all(payload)?;
    let compressed = encoder.finish()?;

    Ok((compressed, oid))
}
