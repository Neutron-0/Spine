//! Tiered composite object store for Spine Storage Engine V2.
//! Combines immutable packfile readers (`.spk` + `.spidx`) with loose object storage.

use crate::catalog::PackCatalog;
use crate::pack::{PackError, PackReader};
use crate::pack_index::PackIndexReader;
use crate::{FileSystemObjectStore, ObjectStore, ObjectStoreError};
use spine_core::{HashAlgorithm, ObjectId, ObjectType};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::RwLock;

#[derive(Debug, Clone, Default)]
pub struct StorageCountSummary {
    pub loose_count: usize,
    pub packed_count: usize,
    pub pack_count: usize,
    pub loose_size_bytes: u64,
    pub packed_size_bytes: u64,
}

#[derive(Debug, Clone, Default)]
pub struct StorageAuditReport {
    pub total_verified_objects: usize,
    pub total_packs_audited: usize,
    pub total_loose_audited: usize,
    pub corrupted_objects: Vec<ObjectId>,
    pub corrupted_packs: Vec<String>,
}

pub struct ActivePack {
    pub id: String,
    pub reader: RwLock<PackReader>,
    pub index: PackIndexReader,
}

pub struct CompositeObjectStore {
    loose: FileSystemObjectStore,
    pack_dir: PathBuf,
    pub(crate) catalog: RwLock<PackCatalog>,
    packs: RwLock<Vec<ActivePack>>,
    algorithm: HashAlgorithm,
}

impl CompositeObjectStore {
    pub fn new<P: AsRef<Path>>(
        spine_dir: P,
        algorithm: HashAlgorithm,
        compression_level: i32,
    ) -> Result<Self, ObjectStoreError> {
        let spine_path = spine_dir.as_ref();
        let loose = FileSystemObjectStore::new(spine_path, algorithm, compression_level)?;
        let pack_dir = spine_path.join("objects").join("pack");
        fs::create_dir_all(&pack_dir)?;

        let catalog = PackCatalog::load_or_empty(&pack_dir)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

        let mut active_packs = Vec::new();
        for meta in &catalog.active_packs {
            let spk_path = pack_dir.join(format!("{}.spk", meta.id));
            let spidx_path = pack_dir.join(format!("{}.spidx", meta.id));
            if spk_path.exists() && spidx_path.exists() {
                match (
                    PackReader::open(&spk_path),
                    PackIndexReader::open(&spidx_path),
                ) {
                    (Ok(reader), Ok(index)) => {
                        active_packs.push(ActivePack {
                            id: meta.id.clone(),
                            reader: RwLock::new(reader),
                            index,
                        });
                    }
                    (Err(e), _) => {
                        eprintln!("Warning: Failed to open pack {}: {}", meta.id, e);
                    }
                    (_, Err(e)) => {
                        eprintln!("Warning: Failed to open index for pack {}: {}", meta.id, e);
                    }
                }
            }
        }

        Ok(Self {
            loose,
            pack_dir,
            catalog: RwLock::new(catalog),
            packs: RwLock::new(active_packs),
            algorithm,
        })
    }

    pub fn pack_dir(&self) -> &Path {
        &self.pack_dir
    }

    pub fn loose_store(&self) -> &FileSystemObjectStore {
        &self.loose
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    pub fn catalog(&self) -> std::sync::RwLockReadGuard<'_, PackCatalog> {
        self.catalog.read().unwrap()
    }

    /// Registers a newly ingested packfile into the catalog, persists catalog.json,
    /// and hot-reloads active pack readers.
    pub fn register_pack(
        &self,
        id: String,
        object_count: u32,
        pack_size_bytes: u64,
        index_size_bytes: u64,
        checksum: [u8; 32],
    ) -> Result<(), ObjectStoreError> {
        let hex_checksum = checksum.iter().map(|b| format!("{:02x}", b)).collect();
        let meta = crate::catalog::PackMetadata {
            id,
            object_count,
            pack_size_bytes,
            index_size_bytes,
            pack_checksum: hex_checksum,
            created_at_nanos: chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0),
        };

        {
            let mut cat = self.catalog.write().unwrap();
            cat.add_pack(meta);
            cat.save_atomic(&self.pack_dir)
                .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;
        }
        self.reload()
    }

    /// Reloads the pack catalog and active pack readers from disk.
    pub fn reload(&self) -> Result<(), ObjectStoreError> {
        let new_catalog = PackCatalog::load_or_empty(&self.pack_dir)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

        let mut new_packs = Vec::new();
        for meta in &new_catalog.active_packs {
            let spk_path = self.pack_dir.join(format!("{}.spk", meta.id));
            let spidx_path = self.pack_dir.join(format!("{}.spidx", meta.id));
            if spk_path.exists() && spidx_path.exists() {
                if let (Ok(r), Ok(idx)) = (
                    PackReader::open(&spk_path),
                    PackIndexReader::open(&spidx_path),
                ) {
                    new_packs.push(ActivePack {
                        id: meta.id.clone(),
                        reader: RwLock::new(r),
                        index: idx,
                    });
                }
            }
        }

        *self.catalog.write().unwrap() = new_catalog;
        *self.packs.write().unwrap() = new_packs;
        Ok(())
    }

    /// Summarizes total counts and disk usage across loose and packed tiers.
    pub fn count_objects(&self) -> StorageCountSummary {
        let mut summary = StorageCountSummary::default();

        // 1. Loose objects
        if let Ok(loose_ids) = self.loose.list_objects() {
            summary.loose_count = loose_ids.len();
            for oid in loose_ids {
                let p = self.loose.object_path(&oid);
                if let Ok(meta) = fs::metadata(&p) {
                    summary.loose_size_bytes += meta.len();
                }
            }
        }

        // 2. Packed objects
        let packs = self.packs.read().unwrap();
        summary.pack_count = packs.len();
        for p in packs.iter() {
            summary.packed_count += p.index.object_count() as usize;
            let spk_path = self.pack_dir.join(format!("{}.spk", p.id));
            if let Ok(meta) = fs::metadata(&spk_path) {
                summary.packed_size_bytes += meta.len();
            }
            let spidx_path = self.pack_dir.join(format!("{}.spidx", p.id));
            if let Ok(meta) = fs::metadata(&spidx_path) {
                summary.packed_size_bytes += meta.len();
            }
        }

        summary
    }

    /// Audits all loose and packed objects for physical bit-rot.
    pub fn verify_storage(&self) -> Result<StorageAuditReport, ObjectStoreError> {
        let mut report = StorageAuditReport::default();

        // 1. Verify packs
        let packs = self.packs.read().unwrap();
        for p in packs.iter() {
            report.total_packs_audited += 1;
            if p.index.verify_integrity().is_err() {
                report.corrupted_packs.push(format!("{}.spidx", p.id));
                continue;
            }
            let mut reader = p.reader.write().unwrap();
            match reader.verify_integrity() {
                Ok(verified) => {
                    report.total_verified_objects += verified as usize;
                }
                Err(_) => {
                    report.corrupted_packs.push(format!("{}.spk", p.id));
                }
            }
        }

        // 2. Verify loose objects
        let loose_ids = self.loose.list_objects()?;
        for oid in loose_ids {
            report.total_loose_audited += 1;
            match self.loose.read_object(&oid) {
                Ok(_) => {
                    report.total_verified_objects += 1;
                }
                Err(_) => {
                    report.corrupted_objects.push(oid);
                }
            }
        }

        Ok(report)
    }
}

impl ObjectStore for CompositeObjectStore {
    fn write_object(
        &self,
        object_type: ObjectType,
        payload: &[u8],
    ) -> Result<ObjectId, ObjectStoreError> {
        // Writes land immediately in loose storage tier for low latency
        self.loose.write_object(object_type, payload)
    }

    fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), ObjectStoreError> {
        // 1. Search active packs (reverse chronological order)
        let packs = self.packs.read().unwrap();
        for pack in packs.iter().rev() {
            if let Some(entry) = pack.index.lookup(id) {
                let mut reader = pack.reader.write().unwrap();
                match reader.read_object_at(entry.offset) {
                    Ok((_oid, obj_type, data)) => return Ok((obj_type, data)),
                    Err(PackError::Io(e)) => return Err(ObjectStoreError::Io(e)),
                    Err(e) => {
                        return Err(ObjectStoreError::Compression(format!(
                            "Pack read error {}: {}",
                            pack.id, e
                        )))
                    }
                }
            }
        }
        drop(packs);

        // 2. Fall back to loose object store
        self.loose.read_object(id)
    }

    fn has_object(&self, id: &ObjectId) -> bool {
        let packs = self.packs.read().unwrap();
        for pack in packs.iter().rev() {
            if pack.index.lookup(id).is_some() {
                return true;
            }
        }
        drop(packs);
        self.loose.has_object(id)
    }

    fn list_objects(&self) -> Result<Vec<ObjectId>, ObjectStoreError> {
        let mut set = HashSet::new();

        // 1. Add all packed IDs
        let packs = self.packs.read().unwrap();
        for p in packs.iter() {
            for oid in p.index.object_ids() {
                set.insert(*oid);
            }
        }
        drop(packs);

        // 2. Add all loose IDs
        for oid in self.loose.list_objects()? {
            set.insert(oid);
        }

        let mut list: Vec<ObjectId> = set.into_iter().collect();
        list.sort();
        Ok(list)
    }
}
