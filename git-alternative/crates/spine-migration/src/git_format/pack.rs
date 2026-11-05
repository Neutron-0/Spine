use super::delta::{apply_delta, DeltaError};
use super::loose::GitObjectType;
use super::oid::GitOid;
use flate2::read::ZlibDecoder;
use sha1::{Digest, Sha1};
use std::collections::HashMap;
use std::io::Read;
use thiserror::Error;

pub const PACK_SIGNATURE: &[u8; 4] = b"PACK";
pub const PACK_VERSION_2: u32 = 2;

#[derive(Error, Debug)]
pub enum PackParseError {
    #[error("Pack data too short: {0} bytes")]
    TooShort(usize),
    #[error("Invalid pack signature: expected PACK")]
    InvalidSignature,
    #[error("Unsupported pack version: {0} (only version 2 is supported)")]
    UnsupportedVersion(u32),
    #[error("Corrupt packfile checksum: expected {expected}, computed {computed}")]
    ChecksumMismatch { expected: GitOid, computed: GitOid },
    #[error("Invalid object type byte in pack: {0}")]
    InvalidType(u8),
    #[error("Zlib decompression error at pack offset {offset}: {source}")]
    ZlibError {
        offset: usize,
        source: std::io::Error,
    },
    #[error("Delta application error for object at offset {offset}: {source}")]
    DeltaError { offset: usize, source: DeltaError },
    #[error("Base object not found for delta at offset {1}: base oid {0}")]
    BaseNotFound(GitOid, usize),
    #[error("Base object not found at pack offset {0} for OFS_DELTA at offset {1}")]
    BaseOffsetNotFound(usize, usize),
    #[error("Unresolved delta chain detected in packfile ({0} objects remaining)")]
    UnresolvedDeltas(usize),
}

#[derive(Debug, Clone)]
pub struct ParsedPackObject {
    pub obj_type: GitObjectType,
    pub payload: Vec<u8>,
    pub oid: GitOid,
}

#[allow(dead_code)]
enum RawPackEntry {
    Resolved {
        obj_type: GitObjectType,
        payload: Vec<u8>,
        oid: GitOid,
    },
    OfsDelta {
        base_offset: usize,
        delta_payload: Vec<u8>,
    },
    RefDelta {
        base_oid: GitOid,
        delta_payload: Vec<u8>,
    },
}

/// Parses an entire Git packfile (v2) from a byte slice, resolving all OFS_DELTA and REF_DELTA objects.
pub fn parse_packfile(pack_data: &[u8]) -> Result<Vec<ParsedPackObject>, PackParseError> {
    parse_packfile_with_resolver(pack_data, |_| None)
}

/// Parses an entire Git packfile (v2) with an external base resolver for thin packs.
pub fn parse_packfile_with_resolver<F>(
    pack_data: &[u8],
    mut base_resolver: F,
) -> Result<Vec<ParsedPackObject>, PackParseError>
where
    F: FnMut(&GitOid) -> Option<(GitObjectType, Vec<u8>)>,
{
    if pack_data.len() < 32 {
        // 12 bytes header + 20 bytes checksum
        return Err(PackParseError::TooShort(pack_data.len()));
    }

    // Verify 20-byte SHA-1 checksum trailer
    let content_len = pack_data.len() - 20;
    let expected_checksum = {
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&pack_data[content_len..]);
        GitOid::from_bytes(bytes)
    };
    let computed_checksum = {
        let mut hasher = Sha1::new();
        hasher.update(&pack_data[..content_len]);
        let res = hasher.finalize();
        let mut bytes = [0u8; 20];
        bytes.copy_from_slice(&res);
        GitOid::from_bytes(bytes)
    };
    if expected_checksum != computed_checksum {
        return Err(PackParseError::ChecksumMismatch {
            expected: expected_checksum,
            computed: computed_checksum,
        });
    }

    // Verify header
    if &pack_data[0..4] != PACK_SIGNATURE {
        return Err(PackParseError::InvalidSignature);
    }
    let version = u32::from_be_bytes([pack_data[4], pack_data[5], pack_data[6], pack_data[7]]);
    if version != PACK_VERSION_2 {
        return Err(PackParseError::UnsupportedVersion(version));
    }
    let num_objects =
        u32::from_be_bytes([pack_data[8], pack_data[9], pack_data[10], pack_data[11]]) as usize;

    let mut cursor = 12;
    let mut entries_by_offset: HashMap<usize, RawPackEntry> = HashMap::with_capacity(num_objects);
    let mut resolved_by_offset: HashMap<usize, (GitObjectType, Vec<u8>, GitOid)> =
        HashMap::with_capacity(num_objects);
    let mut resolved_by_oid: HashMap<GitOid, (GitObjectType, Vec<u8>)> =
        HashMap::with_capacity(num_objects);

    for _ in 0..num_objects {
        let obj_offset = cursor;
        if cursor >= content_len {
            return Err(PackParseError::TooShort(cursor));
        }

        // Parse variable-length header
        let byte0 = pack_data[cursor];
        cursor += 1;

        let type_num = (byte0 >> 4) & 0x07;
        let mut size = (byte0 & 0x0F) as usize;
        let mut shift = 4;
        let mut is_continued = (byte0 & 0x80) != 0;

        while is_continued {
            if cursor >= content_len {
                return Err(PackParseError::TooShort(cursor));
            }
            let b = pack_data[cursor];
            cursor += 1;
            size |= ((b & 0x7F) as usize) << shift;
            shift += 7;
            is_continued = (b & 0x80) != 0;
        }

        match type_num {
            1..=4 => {
                // 1=Commit, 2=Tree, 3=Blob, 4=Tag
                let obj_type = match type_num {
                    1 => GitObjectType::Commit,
                    2 => GitObjectType::Tree,
                    3 => GitObjectType::Blob,
                    4 => GitObjectType::Tag,
                    _ => unreachable!(),
                };

                let mut decoder = ZlibDecoder::new(&pack_data[cursor..content_len]);
                let mut payload = Vec::with_capacity(size);
                decoder
                    .read_to_end(&mut payload)
                    .map_err(|e| PackParseError::ZlibError {
                        offset: cursor,
                        source: e,
                    })?;
                let bytes_consumed = decoder.total_in() as usize;
                cursor += bytes_consumed;

                let oid = GitOid::compute_object(obj_type.as_str(), &payload);
                resolved_by_offset.insert(obj_offset, (obj_type, payload.clone(), oid));
                resolved_by_oid.insert(oid, (obj_type, payload.clone()));
                entries_by_offset.insert(
                    obj_offset,
                    RawPackEntry::Resolved {
                        obj_type,
                        payload,
                        oid,
                    },
                );
            }
            6 => {
                // OFS_DELTA
                if cursor >= content_len {
                    return Err(PackParseError::TooShort(cursor));
                }
                let mut b = pack_data[cursor];
                cursor += 1;
                let mut neg_offset = (b & 0x7F) as usize;
                while (b & 0x80) != 0 {
                    if cursor >= content_len {
                        return Err(PackParseError::TooShort(cursor));
                    }
                    b = pack_data[cursor];
                    cursor += 1;
                    neg_offset = ((neg_offset + 1) << 7) | ((b & 0x7F) as usize);
                }

                let base_offset = obj_offset
                    .checked_sub(neg_offset)
                    .ok_or(PackParseError::BaseOffsetNotFound(neg_offset, obj_offset))?;

                let mut decoder = ZlibDecoder::new(&pack_data[cursor..content_len]);
                let mut delta_payload = Vec::new();
                decoder
                    .read_to_end(&mut delta_payload)
                    .map_err(|e| PackParseError::ZlibError {
                        offset: cursor,
                        source: e,
                    })?;
                let bytes_consumed = decoder.total_in() as usize;
                cursor += bytes_consumed;

                entries_by_offset.insert(
                    obj_offset,
                    RawPackEntry::OfsDelta {
                        base_offset,
                        delta_payload,
                    },
                );
            }
            7 => {
                // REF_DELTA
                if cursor + 20 > content_len {
                    return Err(PackParseError::TooShort(cursor));
                }
                let mut base_oid_bytes = [0u8; 20];
                base_oid_bytes.copy_from_slice(&pack_data[cursor..cursor + 20]);
                let base_oid = GitOid::from_bytes(base_oid_bytes);
                cursor += 20;

                let mut decoder = ZlibDecoder::new(&pack_data[cursor..content_len]);
                let mut delta_payload = Vec::new();
                decoder
                    .read_to_end(&mut delta_payload)
                    .map_err(|e| PackParseError::ZlibError {
                        offset: cursor,
                        source: e,
                    })?;
                let bytes_consumed = decoder.total_in() as usize;
                cursor += bytes_consumed;

                entries_by_offset.insert(
                    obj_offset,
                    RawPackEntry::RefDelta {
                        base_oid,
                        delta_payload,
                    },
                );
            }
            other => return Err(PackParseError::InvalidType(other)),
        }
    }

    // Resolve deltas iteratively until all are resolved
    let mut remaining = entries_by_offset.len() - resolved_by_offset.len();
    let mut progress = true;

    while remaining > 0 && progress {
        progress = false;
        let mut newly_resolved = Vec::new();

        for (&offset, entry) in &entries_by_offset {
            if resolved_by_offset.contains_key(&offset) {
                continue;
            }

            match entry {
                RawPackEntry::OfsDelta {
                    base_offset,
                    delta_payload,
                } => {
                    if let Some((base_type, base_payload, _)) = resolved_by_offset.get(base_offset)
                    {
                        let target_payload = apply_delta(base_payload, delta_payload)
                            .map_err(|e| PackParseError::DeltaError { offset, source: e })?;
                        let oid = GitOid::compute_object(base_type.as_str(), &target_payload);
                        newly_resolved.push((offset, *base_type, target_payload, oid));
                    }
                }
                RawPackEntry::RefDelta {
                    base_oid,
                    delta_payload,
                } => {
                    let base_opt = resolved_by_oid
                        .get(base_oid)
                        .cloned()
                        .or_else(|| base_resolver(base_oid));
                    if let Some((base_type, base_payload)) = base_opt {
                        let target_payload = apply_delta(&base_payload, delta_payload)
                            .map_err(|e| PackParseError::DeltaError { offset, source: e })?;
                        let oid = GitOid::compute_object(base_type.as_str(), &target_payload);
                        newly_resolved.push((offset, base_type, target_payload, oid));
                    }
                }
                RawPackEntry::Resolved { .. } => {}
            }
        }

        if !newly_resolved.is_empty() {
            progress = true;
            for (offset, obj_type, payload, oid) in newly_resolved {
                resolved_by_offset.insert(offset, (obj_type, payload.clone(), oid));
                resolved_by_oid.insert(oid, (obj_type, payload));
                remaining -= 1;
            }
        }
    }

    if remaining > 0 {
        return Err(PackParseError::UnresolvedDeltas(remaining));
    }

    let mut result = Vec::with_capacity(resolved_by_offset.len());
    for (_, (obj_type, payload, oid)) in resolved_by_offset {
        result.push(ParsedPackObject {
            obj_type,
            payload,
            oid,
        });
    }

    Ok(result)
}
