//! # spine-config
//!
//! Configuration parsing, management, and serialization for Spine repositories.

use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum ConfigError {
    #[error("TOML serialization/deserialization error: {0}")]
    Toml(#[from] toml::ser::Error),
    #[error("TOML parse error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Unknown configuration key: {0}")]
    UnknownKey(String),
    #[error("Invalid configuration value for key '{key}': {value}")]
    InvalidValue { key: String, value: String },
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserConfig {
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryConfig {
    pub format_version: u16,
    pub hash_algorithm: String,
    pub compression: String,
}

impl Default for RepositoryConfig {
    fn default() -> Self {
        Self {
            format_version: 1,
            hash_algorithm: "blake3".into(),
            compression: "zstd".into(),
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpineConfig {
    #[serde(default)]
    pub user: UserConfig,
    #[serde(default)]
    pub repository: RepositoryConfig,
}

impl SpineConfig {
    pub fn get_value(&self, key: &str) -> Option<String> {
        match key {
            "user.name" => self.user.name.clone(),
            "user.email" => self.user.email.clone(),
            "repository.format_version" => Some(self.repository.format_version.to_string()),
            "repository.hash_algorithm" => Some(self.repository.hash_algorithm.clone()),
            "repository.compression" => Some(self.repository.compression.clone()),
            _ => None,
        }
    }

    pub fn set_value(&mut self, key: &str, value: &str) -> Result<(), ConfigError> {
        match key {
            "user.name" => {
                self.user.name = Some(value.to_string());
                Ok(())
            }
            "user.email" => {
                self.user.email = Some(value.to_string());
                Ok(())
            }
            "repository.format_version" => {
                let v = value
                    .parse::<u16>()
                    .map_err(|_| ConfigError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    })?;
                self.repository.format_version = v;
                Ok(())
            }
            "repository.hash_algorithm" => {
                if value != "blake3" && value != "sha256" {
                    return Err(ConfigError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                }
                self.repository.hash_algorithm = value.to_string();
                Ok(())
            }
            "repository.compression" => {
                if value != "zstd" {
                    return Err(ConfigError::InvalidValue {
                        key: key.to_string(),
                        value: value.to_string(),
                    });
                }
                self.repository.compression = value.to_string();
                Ok(())
            }
            other => Err(ConfigError::UnknownKey(other.to_string())),
        }
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, ConfigError> {
        let p = path.as_ref();
        if !p.exists() {
            return Ok(Self::default());
        }
        let raw = fs::read_to_string(p)?;
        let cfg: Self = toml::from_str(&raw)?;
        Ok(cfg)
    }

    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), ConfigError> {
        let serialized = toml::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_set_get() {
        let mut cfg = SpineConfig::default();
        cfg.set_value("user.name", "Jane Doe").unwrap();
        cfg.set_value("user.email", "jane@example.com").unwrap();

        assert_eq!(cfg.get_value("user.name").as_deref(), Some("Jane Doe"));
        assert_eq!(
            cfg.get_value("user.email").as_deref(),
            Some("jane@example.com")
        );
        assert!(cfg.set_value("user.unknown", "val").is_err());
    }
}
