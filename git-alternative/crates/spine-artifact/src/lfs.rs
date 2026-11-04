use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LfsError {
    #[error("Invalid LFS pointer format: {0}")]
    InvalidPointer(String),
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Store error: {0}")]
    Store(#[from] crate::store::StoreError),
    #[error("Chunker error: {0}")]
    Chunker(#[from] crate::chunker::ChunkerError),
}

/// A lightweight, immutable reference to a large object stored outside normal repository packfiles.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LfsPointer {
    pub version: u32,
    pub manifest_oid: String,
    pub size_bytes: u64,
    pub object_hash: String,
}

impl LfsPointer {
    pub const HEADER: &'static str = "spine-lfs-v1";

    pub fn new(manifest_oid: String, size_bytes: u64, object_hash: String) -> Self {
        Self {
            version: 1,
            manifest_oid,
            size_bytes,
            object_hash,
        }
    }

    /// Encodes pointer into canonical text representation stored as a normal blob in the VCS tree.
    pub fn encode(&self) -> String {
        format!(
            "{} {} {} {}\n",
            Self::HEADER,
            self.manifest_oid,
            self.size_bytes,
            self.object_hash
        )
    }

    /// Parses pointer from file or blob content.
    pub fn parse(s: &str) -> Result<Self, LfsError> {
        let trimmed = s.trim();
        let parts: Vec<&str> = trimmed.split_whitespace().collect();

        // 1. Concise single-line format: "spine-lfs-v1 <manifest_oid> <size_bytes> <object_hash>"
        if parts.len() == 4 && parts[0] == Self::HEADER {
            let manifest_oid = parts[1].to_string();
            let size_bytes = parts[2]
                .parse::<u64>()
                .map_err(|e| LfsError::InvalidPointer(format!("Invalid size: {}", e)))?;
            let object_hash = parts[3].to_string();
            return Ok(Self {
                version: 1,
                manifest_oid,
                size_bytes,
                object_hash,
            });
        }

        // 2. Multi-line key-value format
        let mut manifest_oid = None;
        let mut size_bytes = None;
        let mut object_hash = None;

        for line in trimmed.lines() {
            let l = line.trim();
            if let Some(val) = l.strip_prefix("oid ") {
                let clean = val
                    .trim_start_matches("spine-blake3:")
                    .trim_start_matches("sha256:")
                    .trim();
                object_hash = Some(clean.to_string());
            } else if let Some(val) = l.strip_prefix("manifest ") {
                manifest_oid = Some(val.trim().to_string());
            } else if let Some(val) = l.strip_prefix("manifest_oid ") {
                manifest_oid = Some(val.trim().to_string());
            } else if let Some(val) = l.strip_prefix("size ") {
                if let Ok(sz) = val.trim().parse::<u64>() {
                    size_bytes = Some(sz);
                }
            }
        }

        if let (Some(m_oid), Some(sz), Some(o_hash)) = (manifest_oid, size_bytes, object_hash) {
            return Ok(Self {
                version: 1,
                manifest_oid: m_oid,
                size_bytes: sz,
                object_hash: o_hash,
            });
        }

        Err(LfsError::InvalidPointer(format!(
            "Could not parse valid LFS pointer from: '{}'",
            if trimmed.len() > 80 {
                &trimmed[..80]
            } else {
                trimmed
            }
        )))
    }

    /// Fast predicate to check if bytes begin with the LFS header.
    pub fn is_lfs_pointer_bytes(bytes: &[u8]) -> bool {
        bytes.starts_with(Self::HEADER.as_bytes())
            || bytes.starts_with(b"version https://")
    }
}

/// Rules configuring which repository files are handled via Large Object Storage.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LfsTrackingRules {
    pub patterns: Vec<String>,
    pub size_threshold_bytes: Option<u64>,
}

impl LfsTrackingRules {
    pub const CONFIG_FILE: &'static str = ".spinelfs";

    pub fn new() -> Self {
        Self::default()
    }

    pub fn track_pattern(&mut self, pattern: &str) -> bool {
        let pat = pattern.trim().to_string();
        if !self.patterns.contains(&pat) {
            self.patterns.push(pat);
            true
        } else {
            false
        }
    }

    pub fn untrack_pattern(&mut self, pattern: &str) -> bool {
        let pat = pattern.trim();
        let initial_len = self.patterns.len();
        self.patterns.retain(|p| p != pat);
        self.patterns.len() < initial_len
    }

    pub fn set_threshold(&mut self, bytes: Option<u64>) {
        self.size_threshold_bytes = bytes;
    }

    /// Evaluates if a relative file path or file size matches tracking rules.
    pub fn is_tracked(&self, rel_path: &str, file_size: u64) -> bool {
        if let Some(threshold) = self.size_threshold_bytes {
            if file_size >= threshold {
                return true;
            }
        }

        let normalized = rel_path.replace('\\', "/");
        for pat in &self.patterns {
            if Self::glob_matches(pat, &normalized) {
                return true;
            }
        }

        false
    }

    fn glob_matches(pattern: &str, path: &str) -> bool {
        let p = pattern.trim();
        if let Some(ext) = p.strip_prefix("*.") {
            return path.ends_with(&format!(".{}", ext));
        }
        if let Some(prefix) = p.strip_suffix("/*") {
            return path.starts_with(prefix);
        }
        if let Some(prefix) = p.strip_suffix("/**") {
            return path.starts_with(prefix);
        }
        p == path
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), std::io::Error> {
        let mut lines = Vec::new();
        if let Some(th) = self.size_threshold_bytes {
            lines.push(format!("threshold={}", th));
        }
        for pat in &self.patterns {
            lines.push(pat.clone());
        }
        fs::write(path, lines.join("\n") + "\n")
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, std::io::Error> {
        let p = path.as_ref();
        if !p.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(p)?;
        let mut rules = Self::default();

        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Some(val) = trimmed.strip_prefix("threshold=") {
                if let Ok(sz) = val.trim().parse::<u64>() {
                    rules.size_threshold_bytes = Some(sz);
                }
            } else {
                rules.track_pattern(trimmed);
            }
        }

        Ok(rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lfs_pointer_roundtrip() {
        let ptr = LfsPointer::new(
            "man_1234567890abcdef".into(),
            104857600,
            "hash_abcdef1234567890".into(),
        );

        let encoded = ptr.encode();
        assert!(LfsPointer::is_lfs_pointer_bytes(encoded.as_bytes()));

        let parsed = LfsPointer::parse(&encoded).expect("Pointer should parse");
        assert_eq!(parsed, ptr);
    }

    #[test]
    fn test_lfs_tracking_rules() {
        let mut rules = LfsTrackingRules::new();
        rules.track_pattern("*.psd");
        rules.track_pattern("videos/*");
        rules.set_threshold(Some(10 * 1024 * 1024)); // 10 MB

        assert!(rules.is_tracked("design/logo.psd", 100));
        assert!(rules.is_tracked("videos/intro.mp4", 500));
        assert!(rules.is_tracked("data/huge.dat", 15 * 1024 * 1024));
        assert!(!rules.is_tracked("src/main.rs", 2048));
    }
}
