//! Conservative reachability-based garbage collection engine for Spine Storage Engine V2.

use crate::composite::CompositeObjectStore;
use crate::{ObjectStore, ObjectStoreError};
use spine_core::{FileMode, ObjectId, ObjectType};
use spine_format::{deserialize_commit, deserialize_tree};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::time::{Duration, SystemTime};

pub const DEFAULT_GRACE_PERIOD_SECS: u64 = 14 * 24 * 3600; // 14 days

#[derive(Debug, Clone, Default)]
pub struct GcSummary {
    pub total_objects_inspected: usize,
    pub reachable_objects: usize,
    pub unreachable_objects: usize,
    pub grace_period_preserved: usize,
    pub pruned_objects: usize,
    pub bytes_reclaimed: u64,
}

/// Computes the complete transitive reachability closure of all objects reachable from `roots`.
pub fn compute_reachable_set(
    store: &CompositeObjectStore,
    roots: &[ObjectId],
) -> Result<HashSet<ObjectId>, ObjectStoreError> {
    let mut reachable = HashSet::new();
    let mut queue = VecDeque::new();

    for r in roots {
        if reachable.insert(*r) {
            queue.push_back(*r);
        }
    }

    while let Some(current_id) = queue.pop_front() {
        if let Ok((obj_type, payload)) = store.read_object(&current_id) {
            match obj_type {
                ObjectType::Commit => {
                    if let Ok(commit) = deserialize_commit(&payload) {
                        if reachable.insert(commit.root_tree) {
                            queue.push_back(commit.root_tree);
                        }
                        for p in commit.parents {
                            if reachable.insert(p) {
                                queue.push_back(p);
                            }
                        }
                    }
                }
                ObjectType::Tree => {
                    if let Ok(entries) = deserialize_tree(&payload) {
                        for e in entries {
                            if reachable.insert(e.object_id) && e.mode == FileMode::Directory {
                                queue.push_back(e.object_id);
                            }
                        }
                    }
                }
                ObjectType::Tag => {
                    if let Ok(tag) = spine_format::deserialize_annotated_tag(&payload) {
                        if reachable.insert(tag.target_object) {
                            queue.push_back(tag.target_object);
                        }
                    }
                }
                ObjectType::Blob => {}
            }
        }
    }

    Ok(reachable)
}

/// Executes conservative garbage collection:
/// - Determines reachability closure from roots.
/// - Identifies unreferenced loose objects.
/// - Enforces grace period threshold.
/// - Safely prunes expired unreferenced loose objects.
pub fn collect_garbage(
    store: &CompositeObjectStore,
    roots: &[ObjectId],
    grace_period: Duration,
    dry_run: bool,
) -> Result<GcSummary, ObjectStoreError> {
    let reachable = compute_reachable_set(store, roots)?;
    let loose = store.loose_store();
    let loose_ids = loose.list_objects()?;

    let mut summary = GcSummary {
        total_objects_inspected: loose_ids.len(),
        reachable_objects: 0,
        unreachable_objects: 0,
        grace_period_preserved: 0,
        pruned_objects: 0,
        bytes_reclaimed: 0,
    };

    let now = SystemTime::now();
    let objects_root = loose.objects_dir.as_path();

    for oid in &loose_ids {
        if reachable.contains(oid) {
            summary.reachable_objects += 1;
            continue;
        }

        summary.unreachable_objects += 1;
        let p = loose.object_path(oid);

        if let Ok(meta) = fs::metadata(&p) {
            let mtime = meta.modified().unwrap_or(now);
            let age = now.duration_since(mtime).unwrap_or(Duration::ZERO);

            if age < grace_period {
                // Preserved to prevent TOCTOU race condition
                summary.grace_period_preserved += 1;
            } else {
                // Candidate for physical deletion
                let size = meta.len();
                summary.pruned_objects += 1;
                summary.bytes_reclaimed += size;

                if !dry_run && fs::remove_file(&p).is_ok() {
                    let rel = format!("{}/{}", &oid.to_hex()[0..2], &oid.to_hex()[2..]);
                    let _ = spine_core::clean_empty_parent_dirs(objects_root, &rel);
                }
            }
        }
    }

    Ok(summary)
}
