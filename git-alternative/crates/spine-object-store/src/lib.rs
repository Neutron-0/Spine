//! # spine-object-store
//!
//! Content-addressed object storage engine for Spine repositories, supporting
//! Zstandard compression, domain-separated hashing, and atomic filesystem transactions.

use spine_core::{HashAlgorithm, ObjectId, ObjectType};
use spine_format::{FormatError, LooseHeader, LOOSE_HEADER_SIZE};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum ObjectStoreError {
    #[error("Object not found: {0}")]
    NotFound(ObjectId),
    #[error("Object hash corruption: expected {expected}, computed {computed}")]
    HashMismatch {
        expected: ObjectId,
        computed: ObjectId,
    },
    #[error("Format error: {0}")]
    Format(#[from] FormatError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Zstandard compression/decompression error: {0}")]
    Compression(String),
}

/// Abstract contract for reading and writing content-addressed objects.
pub trait ObjectStore: Send + Sync {
    /// Writes an object, returning its computed `ObjectId`.
    fn write_object(
        &self,
        object_type: ObjectType,
        payload: &[u8],
    ) -> Result<ObjectId, ObjectStoreError>;

    /// Reads an object by its `ObjectId`, verifying its cryptographic integrity.
    fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), ObjectStoreError>;

    /// Checks if an object exists in the store.
    fn has_object(&self, id: &ObjectId) -> bool;

    /// Lists all object IDs currently present in the store.
    fn list_objects(&self) -> Result<Vec<ObjectId>, ObjectStoreError>;
}

pub mod catalog;
pub mod composite;
pub mod gc;
pub mod lock;
pub mod pack;
pub mod pack_index;
pub mod repack;

pub use catalog::{CatalogError, PackCatalog, PackMetadata};
pub use composite::{ActivePack, CompositeObjectStore, StorageAuditReport, StorageCountSummary};
pub use gc::{collect_garbage, compute_reachable_set, GcSummary, DEFAULT_GRACE_PERIOD_SECS};
pub use lock::{LockError, RepoLock};
pub use pack::{PackError, PackReader, PackWriter, PACK_MAGIC, PACK_VERSION};
pub use pack_index::{
    IndexEntry, IndexEntryMeta, PackIndexError, PackIndexReader, PackIndexWriter, INDEX_MAGIC,
    INDEX_VERSION,
};
pub use repack::{repack_loose_objects, RepackSummary};

/// On-disk filesystem object store located at `.spine/objects`.
pub struct FileSystemObjectStore {
    pub(crate) objects_dir: PathBuf,
    pub(crate) tmp_dir: PathBuf,
    pub(crate) algorithm: HashAlgorithm,
    pub(crate) compression_level: i32,
}

impl FileSystemObjectStore {
    pub fn new<P: AsRef<Path>>(
        spine_dir: P,
        algorithm: HashAlgorithm,
        compression_level: i32,
    ) -> Result<Self, ObjectStoreError> {
        let objects_dir = spine_dir.as_ref().join("objects");
        let tmp_dir = spine_dir.as_ref().join("tmp");

        fs::create_dir_all(&objects_dir)?;
        fs::create_dir_all(&tmp_dir)?;

        Ok(Self {
            objects_dir,
            tmp_dir,
            algorithm,
            compression_level,
        })
    }

    pub fn object_path(&self, id: &ObjectId) -> PathBuf {
        let hex = id.to_hex();
        let (prefix, suffix) = hex.split_at(2);
        self.objects_dir.join(prefix).join(suffix)
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }
}

impl ObjectStore for FileSystemObjectStore {
    fn write_object(
        &self,
        object_type: ObjectType,
        payload: &[u8],
    ) -> Result<ObjectId, ObjectStoreError> {
        let id = ObjectId::compute(self.algorithm, object_type, payload);
        let dest_path = self.object_path(&id);

        if dest_path.exists() {
            return Ok(id);
        }

        let parent_dir = dest_path.parent().unwrap();
        if !parent_dir.exists() {
            fs::create_dir_all(parent_dir)?;
        }

        let header = LooseHeader::new(object_type);
        let header_bytes = header.serialize();

        let compressed = zstd::encode_all(payload, self.compression_level)
            .map_err(|e| ObjectStoreError::Compression(e.to_string()))?;

        let temp_filename = format!("obj-{}", Uuid::new_v4());
        let temp_path = self.tmp_dir.join(temp_filename);

        {
            let mut file = File::create(&temp_path)?;
            file.write_all(&header_bytes)?;
            file.write_all(&compressed)?;
            file.sync_all()?;
        }

        // On Windows, rename can fail if dest_path was created concurrently
        if let Err(e) = fs::rename(&temp_path, &dest_path) {
            if dest_path.exists() {
                let _ = fs::remove_file(&temp_path);
            } else {
                return Err(ObjectStoreError::Io(e));
            }
        }

        Ok(id)
    }

    fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), ObjectStoreError> {
        let path = self.object_path(id);
        if !path.exists() {
            return Err(ObjectStoreError::NotFound(*id));
        }

        let mut file = File::open(&path)?;
        let mut raw = Vec::new();
        file.read_to_end(&mut raw)?;

        if raw.len() < LOOSE_HEADER_SIZE {
            return Err(ObjectStoreError::Format(FormatError::BufferTooShort {
                expected: LOOSE_HEADER_SIZE,
                actual: raw.len(),
            }));
        }

        let header = LooseHeader::deserialize(&raw[..LOOSE_HEADER_SIZE])?;
        let payload = zstd::decode_all(&raw[LOOSE_HEADER_SIZE..])
            .map_err(|e| ObjectStoreError::Compression(e.to_string()))?;

        // Cryptographic integrity verification on read
        let computed = ObjectId::compute(self.algorithm, header.object_type, &payload);
        if computed != *id {
            return Err(ObjectStoreError::HashMismatch {
                expected: *id,
                computed,
            });
        }

        Ok((header.object_type, payload))
    }

    fn has_object(&self, id: &ObjectId) -> bool {
        self.object_path(id).exists()
    }

    fn list_objects(&self) -> Result<Vec<ObjectId>, ObjectStoreError> {
        let mut ids = Vec::new();
        if !self.objects_dir.exists() {
            return Ok(ids);
        }

        for prefix_entry in fs::read_dir(&self.objects_dir)? {
            let prefix_entry = prefix_entry?;
            let prefix_path = prefix_entry.path();
            if prefix_path.is_dir() {
                let prefix_name = prefix_entry.file_name();
                let prefix_str = prefix_name.to_string_lossy();
                if prefix_str.len() == 2 {
                    for obj_entry in fs::read_dir(prefix_path)? {
                        let obj_entry = obj_entry?;
                        let suffix_name = obj_entry.file_name();
                        let suffix_str = suffix_name.to_string_lossy();
                        if suffix_str.len() == 62 {
                            let full_hex = format!("{}{}", prefix_str, suffix_str);
                            if let Ok(oid) = ObjectId::from_hex(&full_hex) {
                                ids.push(oid);
                            }
                        }
                    }
                }
            }
        }
        Ok(ids)
    }
}

/// In-memory object store for high-speed testing and benchmarks.
#[derive(Default)]
pub struct MemoryObjectStore {
    objects: RwLock<HashMap<ObjectId, (ObjectType, Vec<u8>)>>,
    algorithm: HashAlgorithm,
}

impl MemoryObjectStore {
    pub fn new(algorithm: HashAlgorithm) -> Self {
        Self {
            objects: RwLock::new(HashMap::new()),
            algorithm,
        }
    }
}

impl ObjectStore for MemoryObjectStore {
    fn write_object(
        &self,
        object_type: ObjectType,
        payload: &[u8],
    ) -> Result<ObjectId, ObjectStoreError> {
        let id = ObjectId::compute(self.algorithm, object_type, payload);
        let mut store = self.objects.write().unwrap();
        store.insert(id, (object_type, payload.to_vec()));
        Ok(id)
    }

    fn read_object(&self, id: &ObjectId) -> Result<(ObjectType, Vec<u8>), ObjectStoreError> {
        let store = self.objects.read().unwrap();
        match store.get(id) {
            Some((t, data)) => Ok((*t, data.clone())),
            None => Err(ObjectStoreError::NotFound(*id)),
        }
    }

    fn has_object(&self, id: &ObjectId) -> bool {
        let store = self.objects.read().unwrap();
        store.contains_key(id)
    }

    fn list_objects(&self) -> Result<Vec<ObjectId>, ObjectStoreError> {
        let store = self.objects.read().unwrap();
        Ok(store.keys().copied().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_filesystem_object_store_roundtrip() {
        let temp = tempdir().unwrap();
        let store = FileSystemObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        let payload = b"The quick brown fox jumps over the lazy dog";
        let oid = store.write_object(ObjectType::Blob, payload).unwrap();

        assert!(store.has_object(&oid));

        let (t, read_back) = store.read_object(&oid).unwrap();
        assert_eq!(t, ObjectType::Blob);
        assert_eq!(&read_back[..], payload);

        let list = store.list_objects().unwrap();
        assert_eq!(list, vec![oid]);
    }

    #[test]
    fn test_filesystem_object_store_corruption_detection() {
        let temp = tempdir().unwrap();
        let store = FileSystemObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        let payload = b"Original uncorrupted content";
        let oid = store.write_object(ObjectType::Blob, payload).unwrap();

        // Corrupt the file on disk by writing invalid compressed bytes
        let path = store.object_path(&oid);
        let mut raw = fs::read(&path).unwrap();
        let last_idx = raw.len() - 1;
        raw[last_idx] ^= 0xff; // flip bits
        fs::write(&path, raw).unwrap();

        // Read must fail with compression or hash mismatch error
        assert!(store.read_object(&oid).is_err());
    }

    #[test]
    fn test_pack_and_index_roundtrip_with_fanout() {
        let temp = tempdir().unwrap();
        let spk_path = temp.path().join("test.spk");
        let spidx_path = temp.path().join("test.spidx");

        let algo = HashAlgorithm::Blake3;
        let count = 50;
        let mut writer = PackWriter::new(&spk_path, algo, count, 3).unwrap();
        let mut index_entries = Vec::new();
        let mut expected_objects = Vec::new();

        for i in 0..count {
            let payload = format!("Object payload test string iteration number {}", i).into_bytes();
            let oid = ObjectId::compute(algo, ObjectType::Blob, &payload);
            let (offset, stored_len, orig_len) = writer
                .append_object(&oid, ObjectType::Blob, &payload)
                .unwrap();
            index_entries.push(IndexEntry {
                id: oid,
                offset,
                stored_len,
                orig_len,
            });
            expected_objects.push((oid, payload));
        }

        let cum_hash = writer.finish().unwrap();
        let idx_hash = PackIndexWriter::write_index(&spidx_path, algo, index_entries).unwrap();
        assert_ne!(cum_hash, [0u8; 28]);
        assert_ne!(idx_hash, [0u8; 28]);

        // Open reader and verify integrity
        let mut reader = PackReader::open(&spk_path).unwrap();
        assert_eq!(reader.object_count(), count);
        assert_eq!(reader.verify_integrity().unwrap(), count);

        // Open index reader and verify lookup via fanout
        let idx_reader = PackIndexReader::open(&spidx_path).unwrap();
        idx_reader.verify_integrity().unwrap();
        assert_eq!(idx_reader.object_count(), count);

        for (oid, expected_data) in &expected_objects {
            let meta = idx_reader.lookup(oid).expect("Object must be in index");
            let (read_oid, obj_type, data) = reader.read_object_at(meta.offset).unwrap();
            assert_eq!(read_oid, *oid);
            assert_eq!(obj_type, ObjectType::Blob);
            assert_eq!(&data, expected_data);
        }

        // Looking up an unknown object must return None
        let dummy = ObjectId::compute(algo, ObjectType::Blob, b"non-existent object");
        assert!(idx_reader.lookup(&dummy).is_none());
    }

    #[test]
    fn test_repack_loose_to_pack_and_composite_read() {
        let temp = tempdir().unwrap();
        let store = CompositeObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        // Write several loose objects
        let mut oids = Vec::new();
        for i in 0..10 {
            let data = format!("Repack test data content {}", i).into_bytes();
            let oid = store.write_object(ObjectType::Blob, &data).unwrap();
            oids.push((oid, data));
        }

        let pre_summary = store.count_objects();
        assert_eq!(pre_summary.loose_count, 10);
        assert_eq!(pre_summary.pack_count, 0);

        // Execute repack
        let summary = repack_loose_objects(&store, 3, false).unwrap();
        assert_eq!(summary.objects_repacked, 10);
        assert_eq!(summary.loose_deleted, 10);
        assert!(summary.pack_id.is_some());

        let post_summary = store.count_objects();
        assert_eq!(post_summary.loose_count, 0);
        assert_eq!(post_summary.pack_count, 1);
        assert_eq!(post_summary.packed_count, 10);

        // Verify all objects can be read transparently through CompositeObjectStore
        for (oid, data) in &oids {
            assert!(store.has_object(oid));
            let (t, payload) = store.read_object(oid).unwrap();
            assert_eq!(t, ObjectType::Blob);
            assert_eq!(&payload, data);
        }

        // Verify storage audit report
        let report = store.verify_storage().unwrap();
        assert_eq!(report.total_verified_objects, 10);
        assert!(report.corrupted_objects.is_empty());
        assert!(report.corrupted_packs.is_empty());
    }

    #[test]
    fn test_gc_reachability_and_grace_period() {
        let temp = tempdir().unwrap();
        let store = CompositeObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        // Root object (reachable)
        let root_oid = store
            .write_object(ObjectType::Blob, b"root live object")
            .unwrap();
        // Orphan object (unreachable)
        let orphan_oid = store
            .write_object(ObjectType::Blob, b"orphan dead object")
            .unwrap();

        // 1. With a standard 14-day grace period, orphan is unreachable but preserved
        let gc_res = collect_garbage(
            &store,
            &[root_oid],
            std::time::Duration::from_secs(14 * 86400),
            false,
        )
        .unwrap();
        assert_eq!(gc_res.reachable_objects, 1);
        assert_eq!(gc_res.unreachable_objects, 1);
        assert_eq!(gc_res.grace_period_preserved, 1);
        assert_eq!(gc_res.pruned_objects, 0);
        assert!(store.has_object(&orphan_oid));

        // 2. With zero grace period, orphan is immediately pruned
        let gc_prune =
            collect_garbage(&store, &[root_oid], std::time::Duration::ZERO, false).unwrap();
        assert_eq!(gc_prune.pruned_objects, 1);
        assert!(!store.has_object(&orphan_oid));
        assert!(store.has_object(&root_oid));
    }

    #[test]
    fn test_repo_lock_protection() {
        let temp = tempdir().unwrap();
        let lock1 = RepoLock::acquire(temp.path(), "repack").unwrap();
        // Second lock attempt must fail with AlreadyLocked
        assert!(RepoLock::acquire(temp.path(), "gc").is_err());
        drop(lock1);
        // After drop, lock can be acquired again
        let lock2 = RepoLock::acquire(temp.path(), "gc");
        assert!(lock2.is_ok());
    }
}
