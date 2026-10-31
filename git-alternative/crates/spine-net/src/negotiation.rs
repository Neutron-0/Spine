//! Commit DAG negotiation and minimal object delta determination (`WANT`/`HAVE`).

use spine_core::{FileMode, ObjectId, ObjectType};
use spine_format::{deserialize_commit, deserialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore};
use std::collections::{HashSet, VecDeque};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum NegotiationError {
    #[error("Object store error: {0}")]
    Store(#[from] spine_object_store::ObjectStoreError),
    #[error("History traversal error: {0}")]
    History(#[from] spine_history::HistoryError),
    #[error("Format error: {0}")]
    Format(#[from] spine_format::FormatError),
}

/// Gathers the complete set of trees and blobs reachable from a set of commits.
fn gather_reachable_tree_objects(
    store: &CompositeObjectStore,
    commit_ids: &HashSet<ObjectId>,
) -> Result<HashSet<ObjectId>, NegotiationError> {
    let mut tree_objects = HashSet::new();
    let mut tree_queue = VecDeque::new();

    for cid in commit_ids {
        if let Ok((ObjectType::Commit, payload)) = store.read_object(cid) {
            if let Ok(commit) = deserialize_commit(&payload) {
                if tree_objects.insert(commit.root_tree) {
                    tree_queue.push_back(commit.root_tree);
                }
            }
        }
    }

    while let Some(tid) = tree_queue.pop_front() {
        if let Ok((ObjectType::Tree, payload)) = store.read_object(&tid) {
            if let Ok(entries) = deserialize_tree(&payload) {
                for e in entries {
                    if tree_objects.insert(e.object_id) && e.mode == FileMode::Directory {
                        tree_queue.push_back(e.object_id);
                    }
                }
            }
        }
    }

    Ok(tree_objects)
}

/// Computes the minimal delta set of objects needed to satisfy `want` given `have`.
/// Returns an ordered list of ObjectIds (commits, trees, blobs) to transmit.
pub fn compute_missing_objects(
    store: &CompositeObjectStore,
    want: &[ObjectId],
    have: &[ObjectId],
) -> Result<Vec<ObjectId>, NegotiationError> {
    // Expand have set to all known ancestors of have
    let mut have_ancestors = HashSet::new();
    let mut have_queue = VecDeque::new();

    for h in have {
        if store.has_object(h) && have_ancestors.insert(*h) {
            have_queue.push_back(*h);
        }
    }

    while let Some(current) = have_queue.pop_front() {
        if let Ok((ObjectType::Commit, payload)) = store.read_object(&current) {
            if let Ok(commit) = deserialize_commit(&payload) {
                for p in commit.parents {
                    if have_ancestors.insert(p) {
                        have_queue.push_back(p);
                    }
                }
            }
        }
    }

    // Traverse want backwards, stopping at any have_ancestor
    let mut missing_commits = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    for w in want {
        if !have_ancestors.contains(w) && visited.insert(*w) {
            queue.push_back(*w);
        }
    }

    while let Some(current) = queue.pop_front() {
        if have_ancestors.contains(&current) {
            continue;
        }

        if let Ok((ObjectType::Commit, payload)) = store.read_object(&current) {
            missing_commits.push(current);
            if let Ok(commit) = deserialize_commit(&payload) {
                for p in commit.parents {
                    if !have_ancestors.contains(&p) && visited.insert(p) {
                        queue.push_back(p);
                    }
                }
            }
        }
    }

    if missing_commits.is_empty() {
        return Ok(Vec::new());
    }

    let have_trees = gather_reachable_tree_objects(store, &have_ancestors)?;

    let mut result_objects = Vec::new();
    // 1. Commits first
    for c in &missing_commits {
        result_objects.push(*c);
    }

    // 2. Trees and blobs reachable from missing commits that are NOT in have_trees
    let mut tree_queue = VecDeque::new();
    let mut visited_trees = HashSet::new();

    for c in &missing_commits {
        if let Ok((ObjectType::Commit, payload)) = store.read_object(c) {
            if let Ok(commit) = deserialize_commit(&payload) {
                if !have_trees.contains(&commit.root_tree) && visited_trees.insert(commit.root_tree)
                {
                    tree_queue.push_back(commit.root_tree);
                }
            }
        }
    }

    while let Some(tid) = tree_queue.pop_front() {
        result_objects.push(tid);
        if let Ok((ObjectType::Tree, payload)) = store.read_object(&tid) {
            if let Ok(entries) = deserialize_tree(&payload) {
                for e in entries {
                    if !have_trees.contains(&e.object_id) && visited_trees.insert(e.object_id) {
                        if e.mode == FileMode::Directory {
                            tree_queue.push_back(e.object_id);
                        } else {
                            result_objects.push(e.object_id);
                        }
                    }
                }
            }
        }
    }

    Ok(result_objects)
}
