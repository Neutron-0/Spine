//! Repacking and loose-object consolidation engine for Spine Storage Engine V2.

use crate::catalog::PackMetadata;
use crate::composite::CompositeObjectStore;
use crate::pack::{PackReader, PackWriter};
use crate::pack_index::{IndexEntry, PackIndexReader, PackIndexWriter};
use crate::{ObjectStore, ObjectStoreError};
use chrono::Utc;
use spine_core::clean_empty_parent_dirs;
use std::fs;
use uuid::Uuid;

#[derive(Debug, Clone, Default)]
pub struct RepackSummary {
    pub pack_id: Option<String>,
    pub objects_repacked: usize,
    pub original_bytes: u64,
    pub packed_bytes: u64,
    pub index_bytes: u64,
    pub loose_deleted: usize,
}

/// Consolidates all loose objects into a new immutable packfile and companion index.
///
/// Follows the strict Zero-Data-Loss Invariant:
/// 1. Assembles `.tmp-<id>.spk` and `.tmp-<id>.spidx`.
/// 2. Verifies cryptographic integrity of the new pack.
/// 3. Atomically publishes to `catalog.json`.
/// 4. Activates pack in `CompositeObjectStore`.
/// 5. Safely removes migrated loose objects.
pub fn repack_loose_objects(
    store: &CompositeObjectStore,
    compression_level: i32,
    dry_run: bool,
) -> Result<RepackSummary, ObjectStoreError> {
    let loose = store.loose_store();
    let loose_ids = loose.list_objects()?;

    if loose_ids.is_empty() {
        return Ok(RepackSummary::default());
    }

    let mut original_bytes = 0u64;
    for oid in &loose_ids {
        let p = loose.object_path(oid);
        if let Ok(meta) = fs::metadata(&p) {
            original_bytes += meta.len();
        }
    }

    if dry_run {
        return Ok(RepackSummary {
            pack_id: None,
            objects_repacked: loose_ids.len(),
            original_bytes,
            packed_bytes: 0,
            index_bytes: 0,
            loose_deleted: 0,
        });
    }

    let pack_dir = store.pack_dir();
    let pack_uuid = Uuid::new_v4().to_string();
    let pack_id = format!("pack-{}", pack_uuid);

    let tmp_spk = pack_dir.join(format!(".tmp-{}.spk", pack_id));
    let tmp_spidx = pack_dir.join(format!(".tmp-{}.spidx", pack_id));

    let final_spk = pack_dir.join(format!("{}.spk", pack_id));
    let final_spidx = pack_dir.join(format!("{}.spidx", pack_id));

    // 1. Write pack records
    let mut writer = PackWriter::new(
        &tmp_spk,
        loose.algorithm,
        loose_ids.len() as u32,
        compression_level,
    )
    .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

    let mut index_entries = Vec::with_capacity(loose_ids.len());

    for oid in &loose_ids {
        let (obj_type, payload) = loose.read_object(oid)?;
        let (offset, stored_len, orig_len) = writer
            .append_object(oid, obj_type, &payload)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

        index_entries.push(IndexEntry {
            id: *oid,
            offset,
            stored_len,
            orig_len,
        });
    }

    let cum_hash = writer
        .finish()
        .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

    // 2. Write pack index
    let _ = PackIndexWriter::write_index(&tmp_spidx, loose.algorithm, index_entries)
        .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

    // 3. Verify newly generated pack and index before publishing
    {
        let mut r = PackReader::open(&tmp_spk)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;
        r.verify_integrity()
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;

        let idx = PackIndexReader::open(&tmp_spidx)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;
        idx.verify_integrity()
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;
    }

    // 4. Atomic rename into place
    spine_core::atomic_replace_file(&tmp_spk, &final_spk)?;
    spine_core::atomic_replace_file(&tmp_spidx, &final_spidx)?;

    let pack_size_bytes = fs::metadata(&final_spk)?.len();
    let index_size_bytes = fs::metadata(&final_spidx)?.len();

    let hex_checksum = cum_hash
        .iter()
        .map(|b| format!("{:02x}", b))
        .collect::<String>();

    // 5. Update catalog atomically
    {
        let mut catalog = store.catalog.write().unwrap();
        catalog.add_pack(PackMetadata {
            id: pack_id.clone(),
            object_count: loose_ids.len() as u32,
            pack_size_bytes,
            index_size_bytes,
            pack_checksum: hex_checksum,
            created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        });
        catalog
            .save_atomic(pack_dir)
            .map_err(|e| ObjectStoreError::Io(std::io::Error::other(e)))?;
    }

    // 6. Reload store to activate the new pack
    store.reload()?;

    // 7. Verification Gate: ensure every repacked object is readable from the composite store
    for oid in &loose_ids {
        if !store.has_object(oid) {
            return Err(ObjectStoreError::NotFound(*oid));
        }
    }

    // 8. Safely delete loose objects that have been published and verified
    let mut deleted_count = 0;
    let objects_root = loose.objects_dir.as_path();
    for oid in &loose_ids {
        let p = loose.object_path(oid);
        if p.exists() && fs::remove_file(&p).is_ok() {
            deleted_count += 1;
            let rel = format!("{}/{}", &oid.to_hex()[0..2], &oid.to_hex()[2..]);
            let _ = clean_empty_parent_dirs(objects_root, &rel);
        }
    }

    Ok(RepackSummary {
        pack_id: Some(pack_id),
        objects_repacked: loose_ids.len(),
        original_bytes,
        packed_bytes: pack_size_bytes,
        index_bytes: index_size_bytes,
        loose_deleted: deleted_count,
    })
}
