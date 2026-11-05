use super::loose::GitObjectType;
use super::oid::GitOid;
use flate2::write::ZlibEncoder;
use flate2::Compression;
use sha1::{Digest, Sha1};
use std::io::Write;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PackBuildError {
    #[error("I/O error building packfile: {0}")]
    Io(#[from] std::io::Error),
    #[error("Object count exceeds u32 limit: {0}")]
    TooManyObjects(usize),
}

pub struct PackObjectToBuild {
    pub obj_type: GitObjectType,
    pub payload: Vec<u8>,
    pub oid: GitOid,
}

struct ObjectIndexMeta {
    oid: GitOid,
    offset: u32,
    crc32: u32,
}

/// Simple CRC-32 calculator for Git index format.
fn compute_crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFFFFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            if (crc & 1) != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

/// Builds a standard Git packfile (v2) and matching pack index (v2).
/// Returns `(pack_bytes, idx_bytes, pack_oid)`.
pub fn build_pack_and_index(
    objects: Vec<PackObjectToBuild>,
) -> Result<(Vec<u8>, Vec<u8>, GitOid), PackBuildError> {
    if objects.len() > u32::MAX as usize {
        return Err(PackBuildError::TooManyObjects(objects.len()));
    }

    let mut pack_buf = Vec::new();

    // 1. Pack header: PACK + version 2 + count
    pack_buf.extend_from_slice(b"PACK");
    pack_buf.extend_from_slice(&2u32.to_be_bytes());
    pack_buf.extend_from_slice(&(objects.len() as u32).to_be_bytes());

    let mut index_metas = Vec::with_capacity(objects.len());

    // 2. Objects
    for obj in objects {
        let offset = pack_buf.len() as u32;
        let type_num: u8 = match obj.obj_type {
            GitObjectType::Commit => 1,
            GitObjectType::Tree => 2,
            GitObjectType::Blob => 3,
            GitObjectType::Tag => 4,
        };

        // Variable length header
        let mut size = obj.payload.len();
        let mut byte0 = (type_num << 4) | ((size & 0x0F) as u8);
        size >>= 4;
        if size > 0 {
            byte0 |= 0x80;
        }
        pack_buf.push(byte0);

        while size > 0 {
            let mut byte = (size & 0x7F) as u8;
            size >>= 7;
            if size > 0 {
                byte |= 0x80;
            }
            pack_buf.push(byte);
        }

        // Zlib compress payload
        let _compressed_start = pack_buf.len();
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&obj.payload)?;
        let compressed = encoder.finish()?;
        pack_buf.extend_from_slice(&compressed);

        // CRC32 is calculated over header + compressed object data in pack
        let object_slice = &pack_buf[offset as usize..pack_buf.len()];
        let crc = compute_crc32(object_slice);

        index_metas.push(ObjectIndexMeta {
            oid: obj.oid,
            offset,
            crc32: crc,
        });
    }

    // 3. Compute packfile SHA-1 checksum trailer
    let mut pack_hasher = Sha1::new();
    pack_hasher.update(&pack_buf);
    let pack_checksum_res = pack_hasher.finalize();
    let mut pack_checksum = [0u8; 20];
    pack_checksum.copy_from_slice(&pack_checksum_res);
    pack_buf.extend_from_slice(&pack_checksum);

    let pack_oid = GitOid::from_bytes(pack_checksum);

    // 4. Build Index v2 (.idx)
    // Sort index entries by OID lexicographically
    index_metas.sort_by_key(|a| a.oid);

    let mut idx_buf = Vec::new();
    // Magic: \xFFtOc
    idx_buf.extend_from_slice(b"\xFFtOc");
    // Version 2
    idx_buf.extend_from_slice(&2u32.to_be_bytes());

    // Fanout table: 256 entries
    let mut fanout = [0u32; 256];
    for meta in &index_metas {
        let first_byte = meta.oid.as_bytes()[0] as usize;
        for count in &mut fanout[first_byte..256] {
            *count += 1;
        }
    }
    for count in fanout {
        idx_buf.extend_from_slice(&count.to_be_bytes());
    }

    // Table of SHA-1 hashes
    for meta in &index_metas {
        idx_buf.extend_from_slice(meta.oid.as_bytes());
    }

    // Table of CRC-32s
    for meta in &index_metas {
        idx_buf.extend_from_slice(&meta.crc32.to_be_bytes());
    }

    // Table of 4-byte offsets
    for meta in &index_metas {
        idx_buf.extend_from_slice(&meta.offset.to_be_bytes());
    }

    // Packfile SHA-1 checksum
    idx_buf.extend_from_slice(&pack_checksum);

    // Index SHA-1 checksum trailer
    let mut idx_hasher = Sha1::new();
    idx_hasher.update(&idx_buf);
    let idx_checksum_res = idx_hasher.finalize();
    idx_buf.extend_from_slice(&idx_checksum_res);

    Ok((pack_buf, idx_buf, pack_oid))
}
