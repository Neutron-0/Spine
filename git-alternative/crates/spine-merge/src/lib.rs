//! # spine-merge
//!
//! Three-way merge engine, conflict detection, line-level three-way text merging,
//! conflict markers generation, and persistent merge state tracking for Spine VCS.

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
}

/// Result of a 3-way line merge for a text file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextMergeResult {
    Clean(String),
    Conflict(String),
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

    let _diff_ours = compute_line_diff(base, ours);
    let _diff_theirs = compute_line_diff(base, theirs);

    let base_lines: Vec<&str> = base.lines().collect();
    let ours_lines: Vec<&str> = ours.lines().collect();
    let theirs_lines: Vec<&str> = theirs.lines().collect();

    // Line-by-line 3-way merge
    let mut out = String::new();
    let mut has_conflict = false;

    let mut i_b = 0;
    let mut i_o = 0;
    let mut i_t = 0;

    while i_b < base_lines.len() || i_o < ours_lines.len() || i_t < theirs_lines.len() {
        let b_line = base_lines.get(i_b).copied();
        let o_line = ours_lines.get(i_o).copied();
        let t_line = theirs_lines.get(i_t).copied();

        if o_line == t_line {
            if let Some(line) = o_line {
                out.push_str(line);
                out.push('\n');
            }
            if b_line.is_some() {
                i_b += 1;
            }
            i_o += 1;
            i_t += 1;
        } else if o_line == b_line {
            // Ours matches base; take theirs
            if let Some(line) = t_line {
                out.push_str(line);
                out.push('\n');
            }
            if b_line.is_some() {
                i_b += 1;
            }
            if o_line.is_some() {
                i_o += 1;
            }
            i_t += 1;
        } else if t_line == b_line {
            // Theirs matches base; take ours
            if let Some(line) = o_line {
                out.push_str(line);
                out.push('\n');
            }
            if b_line.is_some() {
                i_b += 1;
            }
            i_o += 1;
            if t_line.is_some() {
                i_t += 1;
            }
        } else {
            // Both sides differ from base and from each other -> Conflict region!
            has_conflict = true;
            out.push_str("<<<<<<< OURS\n");
            if let Some(line) = o_line {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str("=======\n");
            if let Some(line) = t_line {
                out.push_str(line);
                out.push('\n');
            }
            out.push_str(">>>>>>> THEIRS\n");

            if b_line.is_some() {
                i_b += 1;
            }
            if o_line.is_some() {
                i_o += 1;
            }
            if t_line.is_some() {
                i_t += 1;
            }
        }
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
