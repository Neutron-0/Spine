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

/// On-disk filesystem object store located at `.spine/objects`.
pub struct FileSystemObjectStore {
    objects_dir: PathBuf,
    tmp_dir: PathBuf,
    algorithm: HashAlgorithm,
    compression_level: i32,
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

    fn object_path(&self, id: &ObjectId) -> PathBuf {
        let hex = id.to_hex();
        let (prefix, suffix) = hex.split_at(2);
        self.objects_dir.join(prefix).join(suffix)
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
}
