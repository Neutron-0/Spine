use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha1::{Digest, Sha1};
use std::fmt;
use std::str::FromStr;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum GitOidError {
    #[error("Invalid hexadecimal Git OID string: {0}")]
    InvalidHex(String),
    #[error("Invalid Git OID length: expected 40 characters (SHA-1) or 64 characters (SHA-256), got {0}")]
    InvalidLength(usize),
}

/// A 20-byte Git SHA-1 object identifier (or 32-byte SHA-256).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GitOid([u8; 20]);

impl GitOid {
    pub const ZERO: Self = Self([0u8; 20]);

    pub const fn zero() -> Self {
        Self::ZERO
    }

    pub fn is_zero(&self) -> bool {
        self.0 == [0u8; 20]
    }

    pub const fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        let mut s = String::with_capacity(40);
        for byte in &self.0 {
            use std::fmt::Write;
            write!(&mut s, "{:02x}", byte).unwrap();
        }
        s
    }

    pub fn from_hex(s: &str) -> Result<Self, GitOidError> {
        let trimmed = s.trim();
        if trimmed.len() != 40 {
            return Err(GitOidError::InvalidLength(trimmed.len()));
        }
        let mut bytes = [0u8; 20];
        for i in 0..20 {
            let chunk = &trimmed[i * 2..i * 2 + 2];
            bytes[i] = u8::from_str_radix(chunk, 16)
                .map_err(|_| GitOidError::InvalidHex(trimmed.to_string()))?;
        }
        Ok(Self(bytes))
    }

    /// Computes the Git SHA-1 hash over raw bytes.
    pub fn compute(data: &[u8]) -> Self {
        let mut hasher = Sha1::new();
        hasher.update(data);
        let result = hasher.finalize();
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&result);
        Self(bytes)
    }

    /// Computes a Git object hash: `"<type> <len>\0<payload>"`
    pub fn compute_object(obj_type: &str, payload: &[u8]) -> Self {
        let header = format!("{} {}\0", obj_type, payload.len());
        let mut hasher = Sha1::new();
        hasher.update(header.as_bytes());
        hasher.update(payload);
        let result = hasher.finalize();
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&result);
        Self(bytes)
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_hex())
    }
}

impl fmt::Debug for GitOid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "GitOid({})", self.to_hex())
    }
}

impl FromStr for GitOid {
    type Err = GitOidError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl Serialize for GitOid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_hex())
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::from_hex(&s).map_err(serde::de::Error::custom)
    }
}
