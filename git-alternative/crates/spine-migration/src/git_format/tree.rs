use super::oid::GitOid;
use std::cmp::Ordering;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum TreeFormatError {
    #[error("Malformed tree entry: unexpected EOF or missing null terminator")]
    MalformedEntry,
    #[error("Invalid octal mode string: {0}")]
    InvalidMode(String),
    #[error("Invalid entry name encoding")]
    InvalidName,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitTreeEntry {
    pub mode: u32,
    pub name: String,
    pub oid: GitOid,
}

impl GitTreeEntry {
    pub fn is_tree(&self) -> bool {
        self.mode == 0o040000 || self.mode == 0o40000
    }

    pub fn is_executable(&self) -> bool {
        self.mode == 0o100755
    }

    pub fn is_symlink(&self) -> bool {
        self.mode == 0o120000
    }

    pub fn is_submodule(&self) -> bool {
        self.mode == 0o160000
    }
}

/// Compares two Git tree entries according to canonical Git sorting rules:
/// Names are compared byte-by-byte, but if an entry is a tree/directory,
/// it is compared as if it had a trailing '/' byte.
pub fn compare_git_tree_entries(a: &GitTreeEntry, b: &GitTreeEntry) -> Ordering {
    let mut a_bytes = a.name.as_bytes().to_vec();
    if a.is_tree() {
        a_bytes.push(b'/');
    }
    let mut b_bytes = b.name.as_bytes().to_vec();
    if b.is_tree() {
        b_bytes.push(b'/');
    }
    a_bytes.cmp(&b_bytes)
}

/// Parses a binary Git tree object payload into a vector of `GitTreeEntry`.
pub fn parse_git_tree(payload: &[u8]) -> Result<Vec<GitTreeEntry>, TreeFormatError> {
    let mut entries = Vec::new();
    let mut cursor = 0;

    while cursor < payload.len() {
        // Find space separating mode and name
        let space_pos = payload[cursor..]
            .iter()
            .position(|&b| b == b' ')
            .ok_or(TreeFormatError::MalformedEntry)?;
        let mode_str = std::str::from_utf8(&payload[cursor..cursor + space_pos])
            .map_err(|_| TreeFormatError::MalformedEntry)?;
        let mode = u32::from_str_radix(mode_str, 8)
            .map_err(|_| TreeFormatError::InvalidMode(mode_str.to_string()))?;
        cursor += space_pos + 1;

        // Find null byte separating name and 20-byte SHA-1
        let null_pos = payload[cursor..]
            .iter()
            .position(|&b| b == 0)
            .ok_or(TreeFormatError::MalformedEntry)?;
        let name = std::str::from_utf8(&payload[cursor..cursor + null_pos])
            .map_err(|_| TreeFormatError::InvalidName)?
            .to_string();
        cursor += null_pos + 1;

        // Read 20-byte SHA-1
        if cursor + 20 > payload.len() {
            return Err(TreeFormatError::MalformedEntry);
        }
        let mut oid_bytes = [0u8; 20];
        oid_bytes.copy_from_slice(&payload[cursor..cursor + 20]);
        cursor += 20;

        entries.push(GitTreeEntry {
            mode,
            name,
            oid: GitOid::from_bytes(oid_bytes),
        });
    }

    Ok(entries)
}

/// Serializes entries into a canonical Git tree object payload, sorted according to Git rules.
pub fn serialize_git_tree(mut entries: Vec<GitTreeEntry>) -> Vec<u8> {
    entries.sort_by(compare_git_tree_entries);
    let mut buf = Vec::new();

    for entry in entries {
        // In Git, mode is formatted in octal without leading zero (e.g. "100644", "40000", "100755")
        let mode_str = format!("{:o}", entry.mode);
        buf.extend_from_slice(mode_str.as_bytes());
        buf.push(b' ');
        buf.extend_from_slice(entry.name.as_bytes());
        buf.push(0);
        buf.extend_from_slice(entry.oid.as_bytes());
    }

    buf
}
