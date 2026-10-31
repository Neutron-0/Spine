//! Multi-pack generational catalog (`catalog.json`) for Spine Storage Engine V2.

use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const CATALOG_FILENAME: &str = "catalog.json";
pub const CATALOG_FORMAT_VERSION: u32 = 1;

#[derive(Error, Debug)]
pub enum CatalogError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Catalog serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Catalog corrupted: {0}")]
    Corrupted(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackMetadata {
    pub id: String,
    pub object_count: u32,
    pub pack_size_bytes: u64,
    pub index_size_bytes: u64,
    pub pack_checksum: String,
    pub created_at_nanos: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackCatalog {
    pub format_version: u32,
    pub generation: u64,
    pub active_packs: Vec<PackMetadata>,
    pub total_objects: u64,
    pub updated_at_nanos: i64,
}

impl Default for PackCatalog {
    fn default() -> Self {
        Self {
            format_version: CATALOG_FORMAT_VERSION,
            generation: 0,
            active_packs: Vec::new(),
            total_objects: 0,
            updated_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        }
    }
}

impl PackCatalog {
    pub fn catalog_path(pack_dir: impl AsRef<Path>) -> PathBuf {
        pack_dir.as_ref().join(CATALOG_FILENAME)
    }

    /// Loads the catalog file if it exists, or initializes an empty generation-0 catalog.
    pub fn load_or_empty(pack_dir: impl AsRef<Path>) -> Result<Self, CatalogError> {
        let path = Self::catalog_path(pack_dir);
        if !path.exists() {
            return Ok(Self::default());
        }

        let bytes = fs::read(&path)?;
        if bytes.is_empty() {
            return Ok(Self::default());
        }

        let catalog: Self = serde_json::from_slice(&bytes)
            .map_err(|e| CatalogError::Corrupted(format!("{}: {}", path.display(), e)))?;
        Ok(catalog)
    }

    /// Atomically persists the catalog to disk via `atomic_write_file`.
    pub fn save_atomic(&self, pack_dir: impl AsRef<Path>) -> Result<(), CatalogError> {
        let path = Self::catalog_path(pack_dir);
        let bytes = serde_json::to_vec_pretty(self)?;
        spine_core::atomic_write_file(&path, &bytes)?;
        Ok(())
    }

    /// Registers a newly created pack, bumping generation and updating totals.
    pub fn add_pack(&mut self, pack: PackMetadata) {
        self.generation += 1;
        self.total_objects += pack.object_count as u64;
        self.updated_at_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.active_packs.push(pack);
    }

    /// Replaces active packs with a consolidated list (used during full repack).
    pub fn replace_packs(&mut self, new_packs: Vec<PackMetadata>) {
        self.generation += 1;
        self.total_objects = new_packs.iter().map(|p| p.object_count as u64).sum();
        self.updated_at_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.active_packs = new_packs;
    }

    /// Removes a pack from active registration.
    pub fn remove_pack(&mut self, pack_id: &str) -> bool {
        if let Some(pos) = self.active_packs.iter().position(|p| p.id == pack_id) {
            let removed = self.active_packs.remove(pos);
            self.generation += 1;
            self.total_objects = self
                .total_objects
                .saturating_sub(removed.object_count as u64);
            self.updated_at_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
            true
        } else {
            false
        }
    }
}
