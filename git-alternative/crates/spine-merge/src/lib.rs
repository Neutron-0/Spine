//! # spine-merge
//!
//! Three-way merge engine, conflict detection, line-level three-way text merging,
//! conflict markers generation, and persistent merge state tracking for Spine VCS.

pub mod replay;
pub use replay::{CherryPickState, RebaseState, RevertState};

use serde::{Deserialize, Serialize};
use spine_core::{FileMode, ObjectId, ObjectType};
use spine_diff::{compute_line_diff, is_binary};
use spine_history::HistoryError;
use spine_object_store::{ObjectStore, ObjectStoreError};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum MergeError {
    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
    #[error("History error: {0}")]
    History(#[from] HistoryError),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("Serialization error: {0}")]
    Toml(#[from] toml::ser::Error),
    #[error("Deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),
    #[error("Merge is already in progress. Resolve or abort before merging.")]
    MergeAlreadyInProgress,
    #[error("No merge in progress to {0}.")]
    NoMergeInProgress(String),
    #[error("Unresolved merge conflicts remain:\n{0}")]
    UnresolvedConflicts(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MergeSide {
    Ours,
    Theirs,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConflictKind {
    ModifyModifyText {
        base: ObjectId,
        ours: ObjectId,
        theirs: ObjectId,
    },
    ModifyModifyBinary {
        base: ObjectId,
        ours: ObjectId,
        theirs: ObjectId,
    },
    DeleteModify {
        base: ObjectId,
        deleted_by: MergeSide,
        modified_id: ObjectId,
    },
    AddAdd {
        ours: ObjectId,
        theirs: ObjectId,
    },
    TypeConflict {
        ours_mode: FileMode,
        theirs_mode: FileMode,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathConflict {
    pub path: String,
    pub kind: ConflictKind,
}

/// Active pending merge state persisted on disk at `.spine/MERGE_STATE`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeState {
    pub source_branch: String,
    pub target_branch: String,
    pub base_commit: ObjectId,
    pub ours_commit: ObjectId,
    pub theirs_commit: ObjectId,
    pub conflicts: Vec<PathConflict>,
    #[serde(default)]
    pub resolved_paths: BTreeSet<String>,
    pub timestamp_nanos: i64,
}

impl MergeState {
    pub fn save_to_file<P: AsRef<Path>>(&self, path: P) -> Result<(), MergeError> {
        let serialized = toml::to_string_pretty(self)?;
        fs::write(path, serialized)?;
        Ok(())
    }

    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self, MergeError> {
        let raw = fs::read_to_string(path)?;
        let state: Self = toml::from_str(&raw)?;
        Ok(state)
    }

    pub fn save_to_dir<P: AsRef<Path>>(&self, spine_dir: P) -> Result<(), MergeError> {
        self.save_to_file(spine_dir.as_ref().join("MERGE_STATE"))
    }

    pub fn load_from_dir<P: AsRef<Path>>(spine_dir: P) -> Result<Option<Self>, MergeError> {
        let p = spine_dir.as_ref().join("MERGE_STATE");
        if !p.exists() {
            return Ok(None);
        }
        let state = Self::load_from_file(p)?;
        Ok(Some(state))
    }

    pub fn clear<P: AsRef<Path>>(spine_dir: P) -> Result<(), MergeError> {
        let p = spine_dir.as_ref().join("MERGE_STATE");
        if p.exists() {
            fs::remove_file(p)?;
        }
        Ok(())
    }

    /// Mark a conflicted path as explicitly resolved.
    /// Returns true if the path was a recognized conflict and was newly marked resolved.
    pub fn mark_resolved(&mut self, path: &str) -> bool {
        if self.conflicts.iter().any(|c| c.path == path) {
            self.resolved_paths.insert(path.to_string())
        } else {
            false
        }
    }

    /// Check whether all recorded conflicts have been explicitly resolved.
    pub fn is_fully_resolved(&self) -> bool {
        self.conflicts
            .iter()
            .all(|c| self.resolved_paths.contains(&c.path))
    }

    /// Returns a list of conflicts that are not yet marked as resolved.
    pub fn unresolved_conflicts(&self) -> Vec<&PathConflict> {
        self.conflicts
            .iter()
            .filter(|c| !self.resolved_paths.contains(&c.path))
            .collect()
    }
}

/// Result of a 3-way line merge for a text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMergeResult {
    Clean(String),
    Conflict(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffHunk {
    base_start: usize,
    base_end: usize,
    replacement: Vec<String>,
}

fn diff_ops_to_hunks(ops: &[spine_diff::DiffOp]) -> Vec<DiffHunk> {
    let mut hunks = Vec::new();
    let mut base_idx = 0;
    let mut i = 0;

    while i < ops.len() {
        match &ops[i] {
            spine_diff::DiffOp::Context(_) => {
                base_idx += 1;
                i += 1;
            }
            spine_diff::DiffOp::Addition(_) | spine_diff::DiffOp::Deletion(_) => {
                let hunk_start = base_idx;
                let mut replacement = Vec::new();

                while i < ops.len() {
                    match &ops[i] {
                        spine_diff::DiffOp::Context(_) => break,
                        spine_diff::DiffOp::Addition(line) => {
                            replacement.push(line.clone());
                            i += 1;
                        }
                        spine_diff::DiffOp::Deletion(_) => {
                            base_idx += 1;
                            i += 1;
                        }
                    }
                }
                hunks.push(DiffHunk {
                    base_start: hunk_start,
                    base_end: base_idx,
                    replacement,
                });
            }
        }
    }

    hunks
}

fn reconstruct_side(
    base_lines: &[&str],
    range_start: usize,
    range_end: usize,
    hunks: &[DiffHunk],
) -> Vec<String> {
    let mut res = Vec::new();
    let mut cur = range_start;
    for h in hunks {
        while cur < h.base_start {
            res.push(base_lines[cur].to_string());
            cur += 1;
        }
        res.extend(h.replacement.clone());
        cur = h.base_end;
    }
    while cur < range_end {
        res.push(base_lines[cur].to_string());
        cur += 1;
    }
    res
}

/// Performs a 3-way line merge between base, ours, and theirs text buffers.
pub fn merge_3way_text(base: &str, ours: &str, theirs: &str) -> TextMergeResult {
    if ours == theirs {
        return TextMergeResult::Clean(ours.to_string());
    }
    if ours == base {
        return TextMergeResult::Clean(theirs.to_string());
    }
    if theirs == base {
        return TextMergeResult::Clean(ours.to_string());
    }

    let diff_ours = compute_line_diff(base, ours);
    let diff_theirs = compute_line_diff(base, theirs);

    let hunks_o = diff_ops_to_hunks(&diff_ours);
    let hunks_t = diff_ops_to_hunks(&diff_theirs);

    let base_lines: Vec<&str> = base.lines().collect();

    let mut out = String::new();
    let mut has_conflict = false;

    let mut idx_o = 0;
    let mut idx_t = 0;
    let mut cursor = 0;

    while cursor < base_lines.len() || idx_o < hunks_o.len() || idx_t < hunks_t.len() {
        let next_o = hunks_o.get(idx_o).map(|h| h.base_start);
        let next_t = hunks_t.get(idx_t).map(|h| h.base_start);

        if next_o.is_none() && next_t.is_none() {
            while cursor < base_lines.len() {
                out.push_str(base_lines[cursor]);
                out.push('\n');
                cursor += 1;
            }
            break;
        }

        let next_start = match (next_o, next_t) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => unreachable!(),
        };

        if next_start > cursor {
            while cursor < next_start && cursor < base_lines.len() {
                out.push_str(base_lines[cursor]);
                out.push('\n');
                cursor += 1;
            }
            cursor = next_start;
        }

        // Group all overlapping hunks at this location
        let group_start = cursor;
        let mut group_end = cursor;
        let mut group_o: Vec<DiffHunk> = Vec::new();
        let mut group_t: Vec<DiffHunk> = Vec::new();

        loop {
            let mut added = false;
            let group_empty = group_o.is_empty() && group_t.is_empty();

            if let Some(ho) = hunks_o.get(idx_o) {
                let overlaps = if group_empty {
                    ho.base_start == cursor
                } else if group_start == group_end {
                    ho.base_start == group_start
                } else {
                    ho.base_start < group_end
                };

                if overlaps {
                    group_end = group_end.max(ho.base_end);
                    group_o.push(ho.clone());
                    idx_o += 1;
                    added = true;
                }
            }

            let group_empty = group_o.is_empty() && group_t.is_empty();
            if let Some(ht) = hunks_t.get(idx_t) {
                let overlaps = if group_empty {
                    ht.base_start == cursor
                } else if group_start == group_end {
                    ht.base_start == group_start
                } else {
                    ht.base_start < group_end
                };

                if overlaps {
                    group_end = group_end.max(ht.base_end);
                    group_t.push(ht.clone());
                    idx_t += 1;
                    added = true;
                }
            }

            if !added {
                break;
            }
        }

        let ours_slice = reconstruct_side(&base_lines, group_start, group_end, &group_o);
        let theirs_slice = reconstruct_side(&base_lines, group_start, group_end, &group_t);

        if group_o.is_empty() && !group_t.is_empty() {
            for l in &theirs_slice {
                out.push_str(l);
                out.push('\n');
            }
        } else if (!group_o.is_empty() && group_t.is_empty()) || ours_slice == theirs_slice {
            for l in &ours_slice {
                out.push_str(l);
                out.push('\n');
            }
        } else {
            has_conflict = true;
            out.push_str("<<<<<<< OURS\n");
            for l in &ours_slice {
                out.push_str(l);
                out.push('\n');
            }
            out.push_str("=======\n");
            for l in &theirs_slice {
                out.push_str(l);
                out.push('\n');
            }
            out.push_str(">>>>>>> THEIRS\n");
        }

        cursor = group_end;
    }

    if has_conflict {
        TextMergeResult::Conflict(out)
    } else {
        TextMergeResult::Clean(out)
    }
}

/// Resolved path during 3-way tree merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedPath {
    KeepOurs,
    TakeTheirs {
        mode: FileMode,
        object_id: ObjectId,
        content: Vec<u8>,
    },
    MergedClean {
        mode: FileMode,
        object_id: ObjectId,
        content: Vec<u8>,
    },
    Delete,
    Conflict(PathConflict),
}

/// Computes the complete 3-way tree merge across BASE, OURS, and THEIRS tree maps.
pub fn merge_3way_trees<S: ObjectStore>(
    store: &S,
    base_tree: &BTreeMap<String, (FileMode, ObjectId)>,
    ours_tree: &BTreeMap<String, (FileMode, ObjectId)>,
    theirs_tree: &BTreeMap<String, (FileMode, ObjectId)>,
) -> Result<BTreeMap<String, ResolvedPath>, MergeError> {
    let mut all_paths = BTreeSet::new();
    for p in base_tree.keys() {
        all_paths.insert(p.clone());
    }
    for p in ours_tree.keys() {
        all_paths.insert(p.clone());
    }
    for p in theirs_tree.keys() {
        all_paths.insert(p.clone());
    }

    let mut resolutions = BTreeMap::new();

    for path in all_paths {
        let b = base_tree.get(&path);
        let o = ours_tree.get(&path);
        let t = theirs_tree.get(&path);

        match (b, o, t) {
            // Case 1: Identical on all three sides
            (Some((_, b_oid)), Some((_, o_oid)), Some((_, t_oid)))
                if b_oid == o_oid && o_oid == t_oid =>
            {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Case 2: Changed only on THEIRS (OURS == BASE)
            (Some((_, b_oid)), Some((_, o_oid)), Some((t_mode, t_oid))) if b_oid == o_oid => {
                let (_, content) = store.read_object(t_oid)?;
                resolutions.insert(
                    path,
                    ResolvedPath::TakeTheirs {
                        mode: *t_mode,
                        object_id: *t_oid,
                        content,
                    },
                );
            }

            // Case 3: Changed only on OURS (THEIRS == BASE)
            (Some((_, b_oid)), Some((_, _o_oid)), Some((_, t_oid))) if b_oid == t_oid => {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Case 4: Identical changes on both sides (OURS == THEIRS != BASE)
            (Some((_, _b_oid)), Some((o_mode, o_oid)), Some((_t_mode, t_oid)))
                if o_oid == t_oid =>
            {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Case 5: Deleted on THEIRS, unchanged on OURS
            (Some((_, b_oid)), Some((_, o_oid)), None) if b_oid == o_oid => {
                resolutions.insert(path, ResolvedPath::Delete);
            }

            // Case 6: Deleted on OURS, unchanged on THEIRS
            (Some((_, b_oid)), None, Some((_, t_oid))) if b_oid == t_oid => {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Case 7: Both deleted
            (Some(_), None, None) => {
                resolutions.insert(path, ResolvedPath::Delete);
            }

            // Case 8: Added only on OURS
            (None, Some((_, _o_oid)), None) => {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Case 9: Added only on THEIRS
            (None, None, Some((t_mode, t_oid))) => {
                let (_, content) = store.read_object(t_oid)?;
                resolutions.insert(
                    path,
                    ResolvedPath::TakeTheirs {
                        mode: *t_mode,
                        object_id: *t_oid,
                        content,
                    },
                );
            }

            // Case 10: Added on both sides with identical content
            (None, Some((_, o_oid)), Some((_, t_oid))) if o_oid == t_oid => {
                resolutions.insert(path, ResolvedPath::KeepOurs);
            }

            // Conflict Case 11: Added on both sides with different content
            (None, Some((_, o_oid)), Some((_, t_oid))) => {
                let (_, o_data) = store.read_object(o_oid)?;
                let (_, t_data) = store.read_object(t_oid)?;

                if is_binary(&o_data) || is_binary(&t_data) {
                    resolutions.insert(
                        path.clone(),
                        ResolvedPath::Conflict(PathConflict {
                            path,
                            kind: ConflictKind::AddAdd {
                                ours: *o_oid,
                                theirs: *t_oid,
                            },
                        }),
                    );
                } else {
                    let o_str = String::from_utf8_lossy(&o_data);
                    let t_str = String::from_utf8_lossy(&t_data);
                    match merge_3way_text("", &o_str, &t_str) {
                        TextMergeResult::Clean(merged) => {
                            let merged_bytes = merged.into_bytes();
                            let new_oid = store.write_object(ObjectType::Blob, &merged_bytes)?;
                            resolutions.insert(
                                path,
                                ResolvedPath::MergedClean {
                                    mode: FileMode::Regular,
                                    object_id: new_oid,
                                    content: merged_bytes,
                                },
                            );
                        }
                        TextMergeResult::Conflict(_) => {
                            resolutions.insert(
                                path.clone(),
                                ResolvedPath::Conflict(PathConflict {
                                    path,
                                    kind: ConflictKind::AddAdd {
                                        ours: *o_oid,
                                        theirs: *t_oid,
                                    },
                                }),
                            );
                        }
                    }
                }
            }

            // Conflict Case 12: Deleted on one side, modified on the other
            (Some((_, b_oid)), None, Some((_, t_oid))) => {
                resolutions.insert(
                    path.clone(),
                    ResolvedPath::Conflict(PathConflict {
                        path,
                        kind: ConflictKind::DeleteModify {
                            base: *b_oid,
                            deleted_by: MergeSide::Ours,
                            modified_id: *t_oid,
                        },
                    }),
                );
            }
            (Some((_, b_oid)), Some((_, o_oid)), None) => {
                resolutions.insert(
                    path.clone(),
                    ResolvedPath::Conflict(PathConflict {
                        path,
                        kind: ConflictKind::DeleteModify {
                            base: *b_oid,
                            deleted_by: MergeSide::Theirs,
                            modified_id: *o_oid,
                        },
                    }),
                );
            }

            // Unreachable fallback: no-op
            (None, None, None) => {}

            // Conflict Case 13: Modified on both sides with different content
            (Some((_, b_oid)), Some((o_mode, o_oid)), Some((_, t_oid))) => {
                let (_, b_data) = store.read_object(b_oid)?;
                let (_, o_data) = store.read_object(o_oid)?;
                let (_, t_data) = store.read_object(t_oid)?;

                if is_binary(&b_data) || is_binary(&o_data) || is_binary(&t_data) {
                    resolutions.insert(
                        path.clone(),
                        ResolvedPath::Conflict(PathConflict {
                            path,
                            kind: ConflictKind::ModifyModifyBinary {
                                base: *b_oid,
                                ours: *o_oid,
                                theirs: *t_oid,
                            },
                        }),
                    );
                } else {
                    let b_str = String::from_utf8_lossy(&b_data);
                    let o_str = String::from_utf8_lossy(&o_data);
                    let t_str = String::from_utf8_lossy(&t_data);

                    match merge_3way_text(&b_str, &o_str, &t_str) {
                        TextMergeResult::Clean(merged) => {
                            let merged_bytes = merged.into_bytes();
                            let new_oid = store.write_object(ObjectType::Blob, &merged_bytes)?;
                            resolutions.insert(
                                path,
                                ResolvedPath::MergedClean {
                                    mode: *o_mode,
                                    object_id: new_oid,
                                    content: merged_bytes,
                                },
                            );
                        }
                        TextMergeResult::Conflict(_) => {
                            resolutions.insert(
                                path.clone(),
                                ResolvedPath::Conflict(PathConflict {
                                    path,
                                    kind: ConflictKind::ModifyModifyText {
                                        base: *b_oid,
                                        ours: *o_oid,
                                        theirs: *t_oid,
                                    },
                                }),
                            );
                        }
                    }
                }
            }
        }
    }

    Ok(resolutions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::HashAlgorithm;
    use spine_object_store::MemoryObjectStore;

    #[test]
    fn test_clean_3way_text_merge_non_overlapping() {
        let base = "line 1\nline 2\nline 3\n";
        let ours = "line 1 modified\nline 2\nline 3\n";
        let theirs = "line 1\nline 2\nline 3 modified\n";

        let result = merge_3way_text(base, ours, theirs);
        match result {
            TextMergeResult::Clean(text) => {
                assert!(text.contains("line 1 modified"));
                assert!(text.contains("line 2"));
                assert!(text.contains("line 3 modified"));
            }
            TextMergeResult::Conflict(_) => panic!("Expected clean merge"),
        }
    }

    #[test]
    fn test_conflicted_3way_text_merge_overlapping() {
        let base = "same header\noriginal line\nsame footer\n";
        let ours = "same header\nours change\nsame footer\n";
        let theirs = "same header\ntheirs change\nsame footer\n";

        let result = merge_3way_text(base, ours, theirs);
        match result {
            TextMergeResult::Conflict(text) => {
                assert!(text.contains("<<<<<<< OURS"));
                assert!(text.contains("ours change"));
                assert!(text.contains("======="));
                assert!(text.contains("theirs change"));
                assert!(text.contains(">>>>>>> THEIRS"));
            }
            TextMergeResult::Clean(_) => panic!("Expected conflict"),
        }
    }

    #[test]
    fn test_tree_merge_independent_additions() {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        let oid_base = store.write_object(ObjectType::Blob, b"base file").unwrap();
        let oid_ours = store.write_object(ObjectType::Blob, b"ours file").unwrap();
        let oid_theirs = store
            .write_object(ObjectType::Blob, b"theirs file")
            .unwrap();

        let mut base_tree = BTreeMap::new();
        base_tree.insert("base.txt".into(), (FileMode::Regular, oid_base));

        let mut ours_tree = base_tree.clone();
        ours_tree.insert("ours.txt".into(), (FileMode::Regular, oid_ours));

        let mut theirs_tree = base_tree.clone();
        theirs_tree.insert("theirs.txt".into(), (FileMode::Regular, oid_theirs));

        let res = merge_3way_trees(&store, &base_tree, &ours_tree, &theirs_tree).unwrap();
        assert_eq!(res.len(), 3);
        assert_eq!(res["base.txt"], ResolvedPath::KeepOurs);
        assert_eq!(res["ours.txt"], ResolvedPath::KeepOurs);
        assert!(matches!(res["theirs.txt"], ResolvedPath::TakeTheirs { .. }));
    }
}
