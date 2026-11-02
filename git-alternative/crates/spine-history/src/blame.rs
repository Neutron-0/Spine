use crate::{get_commit, HistoryError};
use serde::{Deserialize, Serialize};
use spine_core::{FileMode, ObjectId, ObjectType};
use spine_diff::{compute_line_diff, DiffOp};
use spine_object_store::{ObjectStore, ObjectStoreError};
use std::collections::{HashMap, HashSet, VecDeque};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum BlameError {
    #[error("File not found in revision: {0}")]
    FileNotFound(String),
    #[error("History error: {0}")]
    History(#[from] HistoryError),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameLine {
    pub line_number: usize,
    pub commit_id: ObjectId,
    pub author_name: String,
    pub author_email: String,
    pub timestamp_nanos: i64,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlameResult {
    pub path: String,
    pub commit_id: ObjectId,
    pub lines: Vec<BlameLine>,
}

/// Finds a file's blob `ObjectId` given a root tree `ObjectId` and path.
pub fn find_path_in_tree<S: ObjectStore>(
    store: &S,
    tree_id: &ObjectId,
    path: &str,
) -> Result<Option<ObjectId>, HistoryError> {
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return Ok(Some(*tree_id));
    }

    let mut current_tree_id = *tree_id;
    for (i, part) in parts.iter().enumerate() {
        let (obj_type, payload) = store.read_object(&current_tree_id).map_err(|e| match e {
            ObjectStoreError::NotFound(oid) => {
                HistoryError::ObjectStore(ObjectStoreError::NotFound(oid))
            }
            other => HistoryError::ObjectStore(other),
        })?;
        if obj_type != ObjectType::Tree {
            return Ok(None);
        }
        let entries = spine_format::deserialize_tree(&payload)?;
        let matched = entries.into_iter().find(|e| &e.name == part);
        match matched {
            Some(entry) => {
                if i == parts.len() - 1 {
                    return Ok(Some(entry.object_id));
                } else if entry.mode == FileMode::Directory {
                    current_tree_id = entry.object_id;
                } else {
                    return Ok(None);
                }
            }
            None => return Ok(None),
        }
    }
    Ok(None)
}

/// Computes line-by-line blame attribution for a path at the given commit revision.
pub fn compute_blame<S: ObjectStore>(
    store: &S,
    target_commit_id: &ObjectId,
    path: &str,
) -> Result<BlameResult, BlameError> {
    let start_commit = get_commit(store, target_commit_id)?;
    let start_blob_id = find_path_in_tree(store, &start_commit.root_tree, path)?
        .ok_or_else(|| BlameError::FileNotFound(path.to_string()))?;

    let (_, start_blob_bytes) = store.read_object(&start_blob_id)?;
    let file_text = String::from_utf8_lossy(&start_blob_bytes).to_string();
    let lines: Vec<String> = file_text.lines().map(String::from).collect();

    if lines.is_empty() {
        return Ok(BlameResult {
            path: path.to_string(),
            commit_id: *target_commit_id,
            lines: Vec::new(),
        });
    }

    // Line indices 0..N are initially attributed to target_commit_id
    let mut line_commits: Vec<ObjectId> = vec![*target_commit_id; lines.len()];

    // Commits that currently own at least one line and need their parents inspected
    let mut queue = VecDeque::new();
    let mut queued = HashSet::new();
    queue.push_back(*target_commit_id);
    queued.insert(*target_commit_id);

    // Cache of file text per commit to avoid re-reading and re-diffing
    let mut commit_text_cache: HashMap<ObjectId, Option<String>> = HashMap::new();
    commit_text_cache.insert(*target_commit_id, Some(file_text.clone()));

    while let Some(current_commit_id) = queue.pop_front() {
        // Collect line indices currently attributed to current_commit_id
        let owned_indices: Vec<usize> = line_commits
            .iter()
            .enumerate()
            .filter(|(_, &cid)| cid == current_commit_id)
            .map(|(idx, _)| idx)
            .collect();

        if owned_indices.is_empty() {
            continue;
        }

        let current_commit = get_commit(store, &current_commit_id)?;
        if current_commit.parents.is_empty() {
            // Root commit; cannot pass lines further back
            continue;
        }

        let current_text = match commit_text_cache.get(&current_commit_id) {
            Some(Some(t)) => t.clone(),
            _ => {
                let blob_opt = find_path_in_tree(store, &current_commit.root_tree, path)?;
                if let Some(bid) = blob_opt {
                    let (_, bytes) = store.read_object(&bid)?;
                    let t = String::from_utf8_lossy(&bytes).to_string();
                    commit_text_cache.insert(current_commit_id, Some(t.clone()));
                    t
                } else {
                    commit_text_cache.insert(current_commit_id, None);
                    continue;
                }
            }
        };

        // Try to pass blame to parents (checking primary parent first)
        for parent_id in &current_commit.parents {
            let parent_commit = get_commit(store, parent_id)?;
            let parent_blob_id = find_path_in_tree(store, &parent_commit.root_tree, path)?;

            let Some(p_blob_id) = parent_blob_id else {
                continue; // File did not exist in this parent
            };

            let parent_text = match commit_text_cache.get(parent_id) {
                Some(Some(t)) => t.clone(),
                Some(None) => continue,
                None => {
                    let (_, bytes) = store.read_object(&p_blob_id)?;
                    let t = String::from_utf8_lossy(&bytes).to_string();
                    commit_text_cache.insert(*parent_id, Some(t.clone()));
                    t
                }
            };

            // Fast path: identical content
            if parent_text == current_text {
                for &idx in &owned_indices {
                    line_commits[idx] = *parent_id;
                }
                if queued.insert(*parent_id) {
                    queue.push_back(*parent_id);
                }
                break;
            }

            // Line-by-line diff between parent_text and current_text
            let ops = compute_line_diff(&parent_text, &current_text);
            let mut curr_line_idx = 0;
            let mut any_passed = false;

            for op in ops {
                match op {
                    DiffOp::Context(_) => {
                        // This line existed unchanged in both parent and current
                        if line_commits[curr_line_idx] == current_commit_id {
                            line_commits[curr_line_idx] = *parent_id;
                            any_passed = true;
                        }
                        curr_line_idx += 1;
                    }
                    DiffOp::Addition(_) => {
                        // Introduced in current; keep current
                        curr_line_idx += 1;
                    }
                    DiffOp::Deletion(_) => {
                        // Present in parent, deleted in current; ignored
                    }
                }
            }

            if any_passed && queued.insert(*parent_id) {
                queue.push_back(*parent_id);
            }
        }
    }

    // Resolve author metadata for each commit
    let mut commit_metadata = HashMap::new();
    let distinct_commits: HashSet<ObjectId> = line_commits.iter().copied().collect();
    for cid in distinct_commits {
        if let Ok(c) = get_commit(store, &cid) {
            commit_metadata.insert(cid, (c.author_name, c.author_email, c.timestamp_nanos));
        }
    }

    let mut blame_lines = Vec::with_capacity(lines.len());
    for (idx, content) in lines.into_iter().enumerate() {
        let cid = line_commits[idx];
        let (name, email, ts) = commit_metadata
            .get(&cid)
            .cloned()
            .unwrap_or_else(|| ("Unknown".into(), "unknown@spine.org".into(), 0));

        blame_lines.push(BlameLine {
            line_number: idx + 1,
            commit_id: cid,
            author_name: name,
            author_email: email,
            timestamp_nanos: ts,
            content,
        });
    }

    Ok(BlameResult {
        path: path.to_string(),
        commit_id: *target_commit_id,
        lines: blame_lines,
    })
}
