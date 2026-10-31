//! Binary pack index reader and writer (`.spidx`) for Spine Storage Engine V2.

use spine_core::{HashAlgorithm, ObjectId};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const INDEX_MAGIC: &[u8; 4] = b"SPIX";
pub const INDEX_TRAILER_MAGIC: &[u8; 4] = b"SIXT";
pub const INDEX_VERSION: u16 = 1;
pub const INDEX_HEADER_SIZE: usize = 16;
pub const FANOUT_ENTRIES: usize = 256;
pub const FANOUT_SIZE: usize = 256 * 4; // 1024 bytes
pub const INDEX_TRAILER_SIZE: usize = 32;

#[derive(Error, Debug)]
pub enum PackIndexError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid index magic: expected SPIX, got {0:?}")]
    InvalidMagic([u8; 4]),
    #[error("Unsupported index version: {0}")]
    UnsupportedVersion(u16),
    #[error("Invalid hash algorithm: {0}")]
    InvalidHashAlgorithm(u16),
    #[error("Corrupt index header checksum")]
    HeaderChecksumMismatch,
    #[error("Corrupt index trailer magic")]
    InvalidTrailerMagic,
    #[error("Corrupt index cumulative checksum")]
    CumulativeChecksumMismatch,
    #[error("Index file truncated or unexpected EOF")]
    UnexpectedEof,
    #[error("Duplicate or unordered object ID in index table")]
    UnorderedEntries,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexEntryMeta {
    pub offset: u64,
    pub stored_len: u32,
    pub orig_len: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexEntry {
    pub id: ObjectId,
    pub offset: u64,
    pub stored_len: u32,
    pub orig_len: u32,
}

pub struct PackIndexWriter;

impl PackIndexWriter {
    /// Writes a complete `.spidx` file from a collection of index entries.
    pub fn write_index(
        path: impl AsRef<Path>,
        algorithm: HashAlgorithm,
        mut entries: Vec<IndexEntry>,
    ) -> Result<[u8; 28], PackIndexError> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;

        // Sort entries strictly ascending by ObjectId
        entries.sort_by_key(|a| a.id);

        let count = entries.len() as u32;

        // Build 256-bucket fanout table
        let mut fanout = [0u32; FANOUT_ENTRIES];
        for entry in &entries {
            let prefix = entry.id.as_bytes()[0] as usize;
            fanout[prefix] += 1;
        }

        // Convert to cumulative sums
        for i in 1..FANOUT_ENTRIES {
            fanout[i] += fanout[i - 1];
        }

        // 1. Prepare Header
        let mut h_buf = [0u8; 12];
        h_buf[0..4].copy_from_slice(INDEX_MAGIC);
        h_buf[4..6].copy_from_slice(&INDEX_VERSION.to_le_bytes());
        h_buf[6..8].copy_from_slice(&(algorithm as u16).to_le_bytes());
        h_buf[8..12].copy_from_slice(&count.to_le_bytes());

        let h_hash = blake3::hash(&h_buf);
        let mut header = [0u8; INDEX_HEADER_SIZE];
        header[0..12].copy_from_slice(&h_buf);
        header[12..16].copy_from_slice(&h_hash.as_bytes()[0..4]);

        let mut hasher = blake3::Hasher::new();
        file.write_all(&header)?;
        hasher.update(&header);

        // 2. Write Fanout Table
        let mut fanout_bytes = [0u8; FANOUT_SIZE];
        for (i, val) in fanout.iter().enumerate() {
            fanout_bytes[i * 4..i * 4 + 4].copy_from_slice(&val.to_le_bytes());
        }
        file.write_all(&fanout_bytes)?;
        hasher.update(&fanout_bytes);

        // 3. Write Sorted Object IDs
        let mut oids_bytes = Vec::with_capacity(entries.len() * 32);
        for entry in &entries {
            oids_bytes.extend_from_slice(entry.id.as_bytes());
        }
        file.write_all(&oids_bytes)?;
        hasher.update(&oids_bytes);

        // 4. Write Offsets and Lengths
        let mut offsets_bytes = Vec::with_capacity(entries.len() * 16);
        for entry in &entries {
            offsets_bytes.extend_from_slice(&entry.offset.to_le_bytes());
            offsets_bytes.extend_from_slice(&entry.stored_len.to_le_bytes());
            offsets_bytes.extend_from_slice(&entry.orig_len.to_le_bytes());
        }
        file.write_all(&offsets_bytes)?;
        hasher.update(&offsets_bytes);

        // 5. Write Trailer
        let cum_hash = hasher.finalize();
        let mut trailer = [0u8; INDEX_TRAILER_SIZE];
        trailer[0..4].copy_from_slice(INDEX_TRAILER_MAGIC);
        trailer[4..32].copy_from_slice(&cum_hash.as_bytes()[0..28]);

        file.write_all(&trailer)?;
        file.flush()?;
        file.sync_all()?;

        let mut out = [0u8; 28];
        out.copy_from_slice(&cum_hash.as_bytes()[0..28]);
        Ok(out)
    }
}

pub struct PackIndexReader {
    path: PathBuf,
    algorithm: HashAlgorithm,
    object_count: u32,
    fanout: [u32; FANOUT_ENTRIES],
    sorted_ids: Vec<ObjectId>,
    offsets_and_sizes: Vec<IndexEntryMeta>,
    cumulative_hash: [u8; 28],
}

impl PackIndexReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PackIndexError> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let file_len = file.metadata()?.len();

        let min_len = (INDEX_HEADER_SIZE + FANOUT_SIZE + INDEX_TRAILER_SIZE) as u64;
        if file_len < min_len {
            return Err(PackIndexError::UnexpectedEof);
        }

        // 1. Read and verify header
        let mut header = [0u8; INDEX_HEADER_SIZE];
        file.read_exact(&mut header)?;

        if &header[0..4] != INDEX_MAGIC {
            let mut magic = [0u8; 4];
            magic.copy_from_slice(&header[0..4]);
            return Err(PackIndexError::InvalidMagic(magic));
        }

        let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
        if version != INDEX_VERSION {
            return Err(PackIndexError::UnsupportedVersion(version));
        }

        let algo_code = u16::from_le_bytes(header[6..8].try_into().unwrap());
        let algorithm = match algo_code {
            1 => HashAlgorithm::Blake3,
            2 => HashAlgorithm::Sha256,
            other => return Err(PackIndexError::InvalidHashAlgorithm(other)),
        };

        let object_count = u32::from_le_bytes(header[8..12].try_into().unwrap());

        let h_hash = blake3::hash(&header[0..12]);
        if h_hash.as_bytes()[0..4] != header[12..16] {
            return Err(PackIndexError::HeaderChecksumMismatch);
        }

        // Expected total length = 16 + 1024 + (count * 32) + (count * 16) + 32
        let expected_len = min_len + (object_count as u64) * 48;
        if file_len != expected_len {
            return Err(PackIndexError::UnexpectedEof);
        }

        // 2. Read fanout table
        let mut fanout_bytes = [0u8; FANOUT_SIZE];
        file.read_exact(&mut fanout_bytes)?;
        let mut fanout = [0u32; FANOUT_ENTRIES];
        for i in 0..FANOUT_ENTRIES {
            fanout[i] = u32::from_le_bytes(fanout_bytes[i * 4..i * 4 + 4].try_into().unwrap());
        }

        // 3. Read sorted ObjectIds
        let mut oids_bytes = vec![0u8; (object_count as usize) * 32];
        file.read_exact(&mut oids_bytes)?;
        let mut sorted_ids = Vec::with_capacity(object_count as usize);
        for i in 0..(object_count as usize) {
            let chunk = &oids_bytes[i * 32..i * 32 + 32];
            sorted_ids.push(ObjectId::from_bytes(chunk.try_into().unwrap()));
        }

        // 4. Read offsets and sizes
        let mut meta_bytes = vec![0u8; (object_count as usize) * 16];
        file.read_exact(&mut meta_bytes)?;
        let mut offsets_and_sizes = Vec::with_capacity(object_count as usize);
        for i in 0..(object_count as usize) {
            let chunk = &meta_bytes[i * 16..i * 16 + 16];
            let offset = u64::from_le_bytes(chunk[0..8].try_into().unwrap());
            let stored_len = u32::from_le_bytes(chunk[8..12].try_into().unwrap());
            let orig_len = u32::from_le_bytes(chunk[12..16].try_into().unwrap());
            offsets_and_sizes.push(IndexEntryMeta {
                offset,
                stored_len,
                orig_len,
            });
        }

        // 5. Read and verify trailer
        let mut trailer = [0u8; INDEX_TRAILER_SIZE];
        file.read_exact(&mut trailer)?;

        if &trailer[0..4] != INDEX_TRAILER_MAGIC {
            return Err(PackIndexError::InvalidTrailerMagic);
        }

        let mut cumulative_hash = [0u8; 28];
        cumulative_hash.copy_from_slice(&trailer[4..32]);

        Ok(Self {
            path,
            algorithm,
            object_count,
            fanout,
            sorted_ids,
            offsets_and_sizes,
            cumulative_hash,
        })
    }

    pub fn object_count(&self) -> u32 {
        self.object_count
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    pub fn object_ids(&self) -> &[ObjectId] {
        &self.sorted_ids
    }

    /// Looks up an object by ID using 256-bucket fanout and binary search.
    /// Operates in $O(\log_2(N/256))$ comparisons.
    pub fn lookup(&self, id: &ObjectId) -> Option<IndexEntryMeta> {
        if self.object_count == 0 {
            return None;
        }

        let prefix = id.as_bytes()[0] as usize;
        let low = if prefix == 0 {
            0
        } else {
            self.fanout[prefix - 1] as usize
        };
        let high = self.fanout[prefix] as usize;

        if low >= high || high > self.sorted_ids.len() {
            return None;
        }

        let slice = &self.sorted_ids[low..high];
        match slice.binary_search(id) {
            Ok(idx) => Some(self.offsets_and_sizes[low + idx]),
            Err(_) => None,
        }
    }

    /// Verifies the cryptographic cumulative hash of the entire index.
    pub fn verify_integrity(&self) -> Result<(), PackIndexError> {
        let mut file = File::open(&self.path)?;
        let file_len = file.metadata()?.len();
        let payload_len = file_len - (INDEX_TRAILER_SIZE as u64);

        file.seek(SeekFrom::Start(0))?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0u8; 65536];
        let mut remaining = payload_len;

        while remaining > 0 {
            let to_read = (remaining as usize).min(buffer.len());
            let n = file.read(&mut buffer[..to_read])?;
            if n == 0 {
                return Err(PackIndexError::UnexpectedEof);
            }
            hasher.update(&buffer[..n]);
            remaining -= n as u64;
        }

        let cum_hash = hasher.finalize();
        if cum_hash.as_bytes()[0..28] != self.cumulative_hash {
            return Err(PackIndexError::CumulativeChecksumMismatch);
        }

        Ok(())
    }
}
