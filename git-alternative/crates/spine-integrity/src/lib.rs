//! # spine-integrity
//!
//! Repository integrity verification (`spine fsck`), cryptographic hash checking,
//! dangling pointer detection, and structural tree validation.

use spine_core::{FileMode, ObjectId, ObjectType};
use spine_format::{deserialize_commit, deserialize_tree, FormatError};
use spine_index::Index;
use spine_object_store::{ObjectStore, ObjectStoreError};
use std::collections::{HashSet, VecDeque};
use std::fs;
use std::io;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum IntegrityError {
    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
    #[error("Format error: {0}")]
    Format(#[from] FormatError),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    #[error("Index error: {0}")]
    Index(#[from] spine_index::IndexError),
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FsckReport {
    pub objects_checked: usize,
    pub corrupt_objects: Vec<(ObjectId, String)>,
    pub missing_objects: Vec<ObjectId>,
    pub dangling_objects: Vec<ObjectId>,
    pub refs_checked: usize,
    pub invalid_refs: Vec<(String, String)>,
    pub journal_records_checked: usize,
    pub journal_errors: Vec<(String, String)>,
    pub merge_state_errors: Vec<String>,
}

impl FsckReport {
    pub fn is_healthy(&self) -> bool {
        self.corrupt_objects.is_empty()
            && self.missing_objects.is_empty()
            && self.invalid_refs.is_empty()
            && self.journal_errors.is_empty()
            && self.merge_state_errors.is_empty()
    }

    pub fn add_corrupt_object(&mut self, oid: ObjectId, err: String) {
        if !self.corrupt_objects.iter().any(|(o, _)| o == &oid) {
            self.corrupt_objects.push((oid, err));
        }
    }

    pub fn add_missing_object(&mut self, oid: ObjectId) {
        if !self.missing_objects.contains(&oid) {
            self.missing_objects.push(oid);
        }
    }
}

/// Runs a comprehensive integrity check on the repository.
pub fn run_fsck<S: ObjectStore, P: AsRef<Path>>(
    spine_dir: P,
    store: &S,
) -> Result<FsckReport, IntegrityError> {
    let mut report = FsckReport::default();
    let spine_path = spine_dir.as_ref();

    let all_objects = store.list_objects()?;
    let mut reachable_objects = HashSet::new();

    // 1. Verify every object's cryptographic hash on disk
    for oid in &all_objects {
        report.objects_checked += 1;
        match store.read_object(oid) {
            Ok(_) => {}
            Err(e) => {
                report.add_corrupt_object(*oid, e.to_string());
            }
        }
    }

    // 2. Scan and verify references
    let refs_dir = spine_path.join("refs");
    let mut ref_commit_ids = Vec::new();

    if refs_dir.exists() {
        scan_refs_recursive(&refs_dir, &refs_dir, &mut report, &mut ref_commit_ids)?;
    }

    // Check HEAD reference
    let head_path = spine_path.join("HEAD");
    if head_path.exists() {
        report.refs_checked += 1;
        let head_content = fs::read_to_string(&head_path)?.trim().to_string();
        if let Some(target_ref) = head_content.strip_prefix("ref: ") {
            let target_ref_path = spine_path.join(target_ref);
            if !target_ref_path.exists() {
                // Symbolic HEAD pointing to unborn branch is allowed initially if no commits exist
                if !ref_commit_ids.is_empty() {
                    report.invalid_refs.push((
                        "HEAD".into(),
                        format!("Target ref '{}' does not exist", target_ref),
                    ));
                }
            }
        } else if let Ok(direct_oid) = ObjectId::from_hex(&head_content) {
            ref_commit_ids.push(direct_oid);
        } else {
            report
                .invalid_refs
                .push(("HEAD".into(), "Malformed HEAD reference".into()));
        }
    }

    // 3. Graph traversal from all commit references
    let mut commit_queue = VecDeque::new();
    let mut visited_commits = HashSet::new();

    for cid in ref_commit_ids {
        if !store.has_object(&cid) {
            report.add_missing_object(cid);
        } else {
            commit_queue.push_back(cid);
            visited_commits.insert(cid);
            reachable_objects.insert(cid);
        }
    }

    while let Some(cid) = commit_queue.pop_front() {
        match store.read_object(&cid) {
            Ok((ObjectType::Commit, payload)) => {
                match deserialize_commit(&payload) {
                    Ok(commit) => {
                        // Check parents
                        for parent_id in &commit.parents {
                            if !store.has_object(parent_id) {
                                report.add_missing_object(*parent_id);
                            } else if visited_commits.insert(*parent_id) {
                                commit_queue.push_back(*parent_id);
                                reachable_objects.insert(*parent_id);
                            }
                        }

                        // Check root tree
                        check_tree(
                            store,
                            &commit.root_tree,
                            &mut reachable_objects,
                            &mut report,
                        )?;
                    }
                    Err(e) => {
                        report.add_corrupt_object(cid, e.to_string());
                    }
                }
            }
            Ok((other, _)) => {
                report
                    .add_corrupt_object(cid, format!("Expected Commit object, found {:?}", other));
            }
            Err(e) => {
                report.add_corrupt_object(cid, e.to_string());
            }
        }
    }

    // 4. Check index entries
    let index_path = spine_path.join("index");
    if index_path.exists() {
        if let Ok(index) = Index::load_from_file(&index_path) {
            for entry in index.iter() {
                if !store.has_object(&entry.object_id) {
                    report.add_missing_object(entry.object_id);
                } else {
                    reachable_objects.insert(entry.object_id);
                }
            }
        }
    }

    // 5. Identify dangling objects
    for oid in &all_objects {
        if !reachable_objects.contains(oid) {
            report.dangling_objects.push(*oid);
        }
    }

    // 6. Check pending merge state if present
    let merge_state_file = spine_path.join("MERGE_STATE");
    if merge_state_file.exists() {
        match spine_merge::MergeState::load_from_dir(spine_path) {
            Ok(Some(ms)) => {
                if !store.has_object(&ms.ours_commit) {
                    report.merge_state_errors.push(format!(
                        "Merge state references missing ours_commit: {}",
                        ms.ours_commit.to_hex()
                    ));
                }
                if !store.has_object(&ms.theirs_commit) {
                    report.merge_state_errors.push(format!(
                        "Merge state references missing theirs_commit: {}",
                        ms.theirs_commit.to_hex()
                    ));
                }
            }
            Ok(None) => {}
            Err(e) => {
                report
                    .merge_state_errors
                    .push(format!("Corrupt MERGE_STATE: {}", e));
            }
        }
    }

    // 7. Check operation journal integrity
    let journal_dir = spine_path.join("journal");
    if journal_dir.exists() {
        match spine_journal::Journal::new(&journal_dir) {
            Ok(journal) => match journal.list_records() {
                Ok(records) => {
                    report.journal_records_checked = records.len();
                    let mut prev_seq = 0;
                    for r in records {
                        if r.sequence <= prev_seq {
                            report.journal_errors.push((
                                r.id.clone(),
                                format!(
                                    "Non-monotonic sequence number: {} <= {}",
                                    r.sequence, prev_seq
                                ),
                            ));
                        }
                        prev_seq = r.sequence;

                        if let Some(tip) = r.before_state.branch_tip {
                            if !store.has_object(&tip) {
                                report.journal_errors.push((
                                    r.id.clone(),
                                    format!("Before state branch tip missing: {}", tip.to_hex()),
                                ));
                            }
                        }
                    }
                }
                Err(e) => {
                    report
                        .journal_errors
                        .push(("journal".into(), format!("Failed to read records: {}", e)));
                }
            },
            Err(e) => {
                report
                    .journal_errors
                    .push(("journal".into(), format!("Failed to load journal: {}", e)));
            }
        }
    }

    // 8. Audit pack catalog and active packfiles if present
    let pack_dir = spine_path.join("objects").join("pack");
    let catalog_path = pack_dir.join("catalog.json");
    if catalog_path.exists() {
        if let Ok(cat_bytes) = fs::read(&catalog_path) {
            if let Ok(catalog) = serde_json::from_slice::<serde_json::Value>(&cat_bytes) {
                if let Some(active_packs) = catalog.get("active_packs").and_then(|v| v.as_array()) {
                    for p in active_packs {
                        if let Some(id) = p.get("id").and_then(|v| v.as_str()) {
                            let spk = pack_dir.join(format!("{}.spk", id));
                            let spidx = pack_dir.join(format!("{}.spidx", id));
                            if !spk.exists() {
                                report.invalid_refs.push((
                                    format!("pack:{}", id),
                                    "Missing .spk file".to_string(),
                                ));
                            }
                            if !spidx.exists() {
                                report.invalid_refs.push((
                                    format!("pack:{}", id),
                                    "Missing .spidx file".to_string(),
                                ));
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(report)
}

fn scan_refs_recursive(
    base: &Path,
    current: &Path,
    report: &mut FsckReport,
    ref_commits: &mut Vec<ObjectId>,
) -> Result<(), IntegrityError> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            scan_refs_recursive(base, &path, report, ref_commits)?;
        } else {
            report.refs_checked += 1;
            let ref_name = path
                .strip_prefix(base)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();

            let content = fs::read_to_string(&path)?.trim().to_string();
            match ObjectId::from_hex(&content) {
                Ok(oid) => {
                    ref_commits.push(oid);
                }
                Err(_) => {
                    report.invalid_refs.push((
                        ref_name,
                        format!("Reference contains invalid hash '{}'", content),
                    ));
                }
            }
        }
    }
    Ok(())
}

fn check_tree<S: ObjectStore>(
    store: &S,
    tree_id: &ObjectId,
    reachable: &mut HashSet<ObjectId>,
    report: &mut FsckReport,
) -> Result<(), IntegrityError> {
    reachable.insert(*tree_id);

    if !store.has_object(tree_id) {
        report.add_missing_object(*tree_id);
        return Ok(());
    }

    match store.read_object(tree_id) {
        Ok((ObjectType::Tree, payload)) => match deserialize_tree(&payload) {
            Ok(entries) => {
                for entry in entries {
                    reachable.insert(entry.object_id);
                    if !store.has_object(&entry.object_id) {
                        report.add_missing_object(entry.object_id);
                    } else if entry.mode == FileMode::Directory {
                        check_tree(store, &entry.object_id, reachable, report)?;
                    }
                }
            }
            Err(e) => {
                report.add_corrupt_object(*tree_id, e.to_string());
            }
        },
        Ok((other, _)) => {
            report.add_corrupt_object(*tree_id, format!("Expected Tree object, found {:?}", other));
        }
        Err(e) => {
            report.add_corrupt_object(*tree_id, e.to_string());
        }
    }

    Ok(())
}
