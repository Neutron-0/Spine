//! Binary packfile reader and writer (`.spk`) for Spine Storage Engine V2.

use spine_core::{HashAlgorithm, ObjectId, ObjectType};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const PACK_MAGIC: &[u8; 4] = b"SPPK";
pub const PACK_TRAILER_MAGIC: &[u8; 4] = b"SPTR";
pub const PACK_VERSION: u16 = 1;
pub const PACK_HEADER_SIZE: usize = 16;
pub const RECORD_HEADER_SIZE: usize = 68;
pub const PACK_TRAILER_SIZE: usize = 32;
pub const MAX_OBJECT_SIZE: u64 = 4 * 1024 * 1024 * 1024; // 4 GB safety ceiling

#[derive(Error, Debug)]
pub enum PackError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Invalid packfile magic: expected SPPK, got {0:?}")]
    InvalidMagic([u8; 4]),
    #[error("Unsupported packfile version: {0}")]
    UnsupportedVersion(u16),
    #[error("Invalid hash algorithm identifier: {0}")]
    InvalidHashAlgorithm(u16),
    #[error("Corrupt pack header checksum")]
    HeaderChecksumMismatch,
    #[error("Corrupt pack trailer magic")]
    InvalidTrailerMagic,
    #[error("Corrupt pack cumulative checksum")]
    CumulativeChecksumMismatch,
    #[error("Record payload checksum mismatch for object {0}")]
    PayloadChecksumMismatch(ObjectId),
    #[error("Object size exceeds safety ceiling: {0} bytes")]
    ObjectTooLarge(u64),
    #[error("Decompression failed: {0}")]
    Decompression(String),
    #[error("Compression failed: {0}")]
    Compression(String),
    #[error("Cryptographic hash mismatch: expected {expected}, computed {computed}")]
    HashMismatch {
        expected: ObjectId,
        computed: ObjectId,
    },
    #[error("Invalid record type: {0}")]
    InvalidRecordType(u8),
    #[error("Packfile truncated or unexpected EOF")]
    UnexpectedEof,
}

pub struct PackWriter {
    path: PathBuf,
    file: File,
    hasher: blake3::Hasher,
    algorithm: HashAlgorithm,
    compression_level: i32,
    object_count: u32,
    bytes_written: u64,
}

impl PackWriter {
    pub fn new(
        path: impl AsRef<Path>,
        algorithm: HashAlgorithm,
        object_count: u32,
        compression_level: i32,
    ) -> Result<Self, PackError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&path)?;

        let mut h_buf = [0u8; 12];
        h_buf[0..4].copy_from_slice(PACK_MAGIC);
        h_buf[4..6].copy_from_slice(&PACK_VERSION.to_le_bytes());
        h_buf[6..8].copy_from_slice(&(algorithm as u16).to_le_bytes());
        h_buf[8..12].copy_from_slice(&object_count.to_le_bytes());

        let h_hash = blake3::hash(&h_buf);
        let h_check = &h_hash.as_bytes()[0..4];

        let mut full_header = [0u8; PACK_HEADER_SIZE];
        full_header[0..12].copy_from_slice(&h_buf);
        full_header[12..16].copy_from_slice(h_check);

        file.write_all(&full_header)?;

        let mut hasher = blake3::Hasher::new();
        hasher.update(&full_header);

        Ok(Self {
            path,
            file,
            hasher,
            algorithm,
            compression_level,
            object_count,
            bytes_written: PACK_HEADER_SIZE as u64,
        })
    }

    /// Appends an object record to the packfile.
    /// Returns `(pack_offset, stored_len, orig_len)`.
    pub fn append_object(
        &mut self,
        id: &ObjectId,
        object_type: ObjectType,
        uncompressed: &[u8],
    ) -> Result<(u64, u32, u32), PackError> {
        let orig_len = uncompressed.len() as u64;
        if orig_len > MAX_OBJECT_SIZE {
            return Err(PackError::ObjectTooLarge(orig_len));
        }

        // Try Zstandard compression; if payload is small (< 64B) or incompressible, use raw
        let (compression_mode, stored_payload) = if orig_len < 64 {
            (0u8, uncompressed.to_vec())
        } else {
            let compressed = zstd::encode_all(uncompressed, self.compression_level)
                .map_err(|e| PackError::Compression(e.to_string()))?;
            if compressed.len() >= uncompressed.len() {
                (0u8, uncompressed.to_vec())
            } else {
                (1u8, compressed)
            }
        };

        let stored_len = stored_payload.len() as u64;
        let p_hash = blake3::hash(&stored_payload);

        let mut rec_hdr = [0u8; RECORD_HEADER_SIZE];
        rec_hdr[0] = object_type.as_u8();
        rec_hdr[1] = compression_mode;
        rec_hdr[2..4].copy_from_slice(&[0u8, 0u8]); // Reserved
        rec_hdr[4..12].copy_from_slice(&orig_len.to_le_bytes());
        rec_hdr[12..20].copy_from_slice(&stored_len.to_le_bytes());
        rec_hdr[20..52].copy_from_slice(id.as_bytes());
        rec_hdr[52..68].copy_from_slice(&p_hash.as_bytes()[0..16]);

        let offset = self.bytes_written;

        self.file.write_all(&rec_hdr)?;
        self.file.write_all(&stored_payload)?;

        self.hasher.update(&rec_hdr);
        self.hasher.update(&stored_payload);

        let delta = RECORD_HEADER_SIZE as u64 + stored_len;
        self.bytes_written += delta;

        Ok((offset, stored_len as u32, orig_len as u32))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    pub fn object_count(&self) -> u32 {
        self.object_count
    }

    /// Finalizes the packfile by writing the 32-byte trailer and syncing to disk.
    pub fn finish(mut self) -> Result<[u8; 28], PackError> {
        let cum_hash = self.hasher.finalize();
        let mut trailer = [0u8; PACK_TRAILER_SIZE];
        trailer[0..4].copy_from_slice(PACK_TRAILER_MAGIC);
        trailer[4..32].copy_from_slice(&cum_hash.as_bytes()[0..28]);

        self.file.write_all(&trailer)?;
        self.file.flush()?;
        self.file.sync_all()?;

        let mut out = [0u8; 28];
        out.copy_from_slice(&cum_hash.as_bytes()[0..28]);
        Ok(out)
    }
}

pub struct PackReader {
    path: PathBuf,
    file: File,
    algorithm: HashAlgorithm,
    object_count: u32,
    cumulative_hash: [u8; 28],
}

impl PackReader {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, PackError> {
        let path = path.as_ref().to_path_buf();
        let mut file = File::open(&path)?;
        let file_len = file.metadata()?.len();

        if file_len < (PACK_HEADER_SIZE + PACK_TRAILER_SIZE) as u64 {
            return Err(PackError::UnexpectedEof);
        }

        let mut header = [0u8; PACK_HEADER_SIZE];
        file.read_exact(&mut header)?;

        if &header[0..4] != PACK_MAGIC {
            let mut magic = [0u8; 4];
            magic.copy_from_slice(&header[0..4]);
            return Err(PackError::InvalidMagic(magic));
        }

        let version = u16::from_le_bytes(header[4..6].try_into().unwrap());
        if version != PACK_VERSION {
            return Err(PackError::UnsupportedVersion(version));
        }

        let algo_code = u16::from_le_bytes(header[6..8].try_into().unwrap());
        let algorithm = match algo_code {
            1 => HashAlgorithm::Blake3,
            2 => HashAlgorithm::Sha256,
            other => return Err(PackError::InvalidHashAlgorithm(other)),
        };

        let object_count = u32::from_le_bytes(header[8..12].try_into().unwrap());

        let h_hash = blake3::hash(&header[0..12]);
        if h_hash.as_bytes()[0..4] != header[12..16] {
            return Err(PackError::HeaderChecksumMismatch);
        }

        // Verify trailer
        file.seek(SeekFrom::End(-(PACK_TRAILER_SIZE as i64)))?;
        let mut trailer = [0u8; PACK_TRAILER_SIZE];
        file.read_exact(&mut trailer)?;

        if &trailer[0..4] != PACK_TRAILER_MAGIC {
            return Err(PackError::InvalidTrailerMagic);
        }

        let mut cumulative_hash = [0u8; 28];
        cumulative_hash.copy_from_slice(&trailer[4..32]);

        Ok(Self {
            path,
            file,
            algorithm,
            object_count,
            cumulative_hash,
        })
    }

    pub fn object_count(&self) -> u32 {
        self.object_count
    }

    pub fn algorithm(&self) -> HashAlgorithm {
        self.algorithm
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reads and verifies an object stored at a specific byte offset in the packfile.
    pub fn read_object_at(
        &mut self,
        offset: u64,
    ) -> Result<(ObjectId, ObjectType, Vec<u8>), PackError> {
        self.file.seek(SeekFrom::Start(offset))?;

        let mut rec_hdr = [0u8; RECORD_HEADER_SIZE];
        self.file.read_exact(&mut rec_hdr)?;

        let obj_type_byte = rec_hdr[0];
        let object_type = ObjectType::from_u8(obj_type_byte)
            .map_err(|_| PackError::InvalidRecordType(obj_type_byte))?;

        let compression_mode = rec_hdr[1];
        let orig_len = u64::from_le_bytes(rec_hdr[4..12].try_into().unwrap());
        if orig_len > MAX_OBJECT_SIZE {
            return Err(PackError::ObjectTooLarge(orig_len));
        }

        let stored_len = u64::from_le_bytes(rec_hdr[12..20].try_into().unwrap());
        if stored_len > MAX_OBJECT_SIZE {
            return Err(PackError::ObjectTooLarge(stored_len));
        }

        let expected_oid = ObjectId::from_bytes(rec_hdr[20..52].try_into().unwrap());
        let expected_payload_check = &rec_hdr[52..68];

        let mut stored_payload = vec![0u8; stored_len as usize];
        self.file.read_exact(&mut stored_payload)?;

        // Verify payload checksum
        let p_hash = blake3::hash(&stored_payload);
        if &p_hash.as_bytes()[0..16] != expected_payload_check {
            return Err(PackError::PayloadChecksumMismatch(expected_oid));
        }

        let uncompressed = match compression_mode {
            0 => stored_payload,
            1 => zstd::decode_all(&stored_payload[..])
                .map_err(|e| PackError::Decompression(e.to_string()))?,
            other => {
                return Err(PackError::Decompression(format!(
                    "Unknown compression mode: {}",
                    other
                )))
            }
        };

        // Full cryptographic domain-separated hash verification
        let computed = ObjectId::compute(self.algorithm, object_type, &uncompressed);
        if computed != expected_oid {
            return Err(PackError::HashMismatch {
                expected: expected_oid,
                computed,
            });
        }

        Ok((expected_oid, object_type, uncompressed))
    }

    /// Audits the entire packfile for bit-rot, verifying the cumulative hash
    /// and every object record payload.
    pub fn verify_integrity(&mut self) -> Result<u32, PackError> {
        let file_len = self.file.metadata()?.len();
        let payload_len = file_len - (PACK_TRAILER_SIZE as u64);

        self.file.seek(SeekFrom::Start(0))?;
        let mut hasher = blake3::Hasher::new();

        let mut buffer = [0u8; 65536];
        let mut remaining = payload_len;

        while remaining > 0 {
            let to_read = (remaining as usize).min(buffer.len());
            let n = self.file.read(&mut buffer[..to_read])?;
            if n == 0 {
                return Err(PackError::UnexpectedEof);
            }
            hasher.update(&buffer[..n]);
            remaining -= n as u64;
        }

        let cum_hash = hasher.finalize();
        if cum_hash.as_bytes()[0..28] != self.cumulative_hash {
            return Err(PackError::CumulativeChecksumMismatch);
        }

        // Now walk sequential records to verify each one
        let mut current_offset = PACK_HEADER_SIZE as u64;
        let mut verified_count = 0;

        while current_offset < payload_len && verified_count < self.object_count {
            let (_, _, _) = self.read_object_at(current_offset)?;
            // Compute next record offset
            self.file.seek(SeekFrom::Start(current_offset + 12))?;
            let mut len_buf = [0u8; 8];
            self.file.read_exact(&mut len_buf)?;
            let stored_len = u64::from_le_bytes(len_buf);
            current_offset += (RECORD_HEADER_SIZE as u64) + stored_len;
            verified_count += 1;
        }

        Ok(verified_count)
    }

    /// Scans the packfile and extracts metadata entries to construct a `.spidx` index.
    pub fn scan_index_entries(&mut self) -> Result<Vec<crate::pack_index::IndexEntry>, PackError> {
        let file_len = self.file.metadata()?.len();
        let payload_len = file_len - (PACK_TRAILER_SIZE as u64);
        let mut current_offset = PACK_HEADER_SIZE as u64;
        let mut entries = Vec::with_capacity(self.object_count as usize);

        while current_offset < payload_len && entries.len() < self.object_count as usize {
            self.file.seek(SeekFrom::Start(current_offset))?;
            let mut rec_hdr = [0u8; RECORD_HEADER_SIZE];
            self.file.read_exact(&mut rec_hdr)?;

            let orig_len = u64::from_le_bytes(rec_hdr[4..12].try_into().unwrap());
            let stored_len = u64::from_le_bytes(rec_hdr[12..20].try_into().unwrap());
            let oid = ObjectId::from_bytes(rec_hdr[20..52].try_into().unwrap());

            entries.push(crate::pack_index::IndexEntry {
                id: oid,
                offset: current_offset,
                stored_len: stored_len as u32,
                orig_len: orig_len as u32,
            });

            current_offset += (RECORD_HEADER_SIZE as u64) + stored_len;
        }

        Ok(entries)
    }
}
