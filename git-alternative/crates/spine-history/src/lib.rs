//! # spine-history
//!
//! Commit graph navigation, ancestor discovery, and topological history traversal.

use spine_core::{Commit, ObjectId, ObjectType};
use spine_format::{deserialize_commit, FormatError};
use spine_object_store::{ObjectStore, ObjectStoreError};
use std::collections::{HashSet, VecDeque};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum HistoryError {
    #[error("Commit object not found: {0}")]
    CommitNotFound(ObjectId),
    #[error("Object is not a commit: {0}")]
    NotACommit(ObjectId),
    #[error("Object store error: {0}")]
    ObjectStore(#[from] ObjectStoreError),
    #[error("Format error: {0}")]
    Format(#[from] FormatError),
}

/// A node in the commit log containing both its `ObjectId` and deserialized metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitRecord {
    pub id: ObjectId,
    pub commit: Commit,
}

/// Reads and parses a commit from the object store.
pub fn get_commit<S: ObjectStore>(store: &S, id: &ObjectId) -> Result<Commit, HistoryError> {
    let (obj_type, payload) = store.read_object(id).map_err(|e| match e {
        ObjectStoreError::NotFound(oid) => HistoryError::CommitNotFound(oid),
        other => HistoryError::ObjectStore(other),
    })?;

    if obj_type != ObjectType::Commit {
        return Err(HistoryError::NotACommit(*id));
    }

    let commit = deserialize_commit(&payload)?;
    Ok(commit)
}

/// Traverses the commit graph from the starting commit backwards, returning commits in topological order.
pub fn walk_history<S: ObjectStore>(
    store: &S,
    start_id: &ObjectId,
    limit: Option<usize>,
) -> Result<Vec<CommitRecord>, HistoryError> {
    let mut results = Vec::new();
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(*start_id);
    visited.insert(*start_id);

    while let Some(current_id) = queue.pop_front() {
        let commit = get_commit(store, &current_id)?;

        for parent in &commit.parents {
            if visited.insert(*parent) {
                queue.push_back(*parent);
            }
        }

        results.push(CommitRecord {
            id: current_id,
            commit,
        });

        if let Some(max) = limit {
            if results.len() >= max {
                break;
            }
        }
    }

    Ok(results)
}

/// Checks if `maybe_ancestor` is reachable by following parent links from `descendant`.
pub fn is_ancestor<S: ObjectStore>(
    store: &S,
    maybe_ancestor: &ObjectId,
    descendant: &ObjectId,
) -> Result<bool, HistoryError> {
    if maybe_ancestor == descendant {
        return Ok(true);
    }

    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(*descendant);
    visited.insert(*descendant);

    while let Some(current) = queue.pop_front() {
        if &current == maybe_ancestor {
            return Ok(true);
        }

        let commit = get_commit(store, &current)?;
        for parent in &commit.parents {
            if visited.insert(*parent) {
                queue.push_back(*parent);
            }
        }
    }

    Ok(false)
}

/// Computes the set of ancestors (including the commit itself) for a given commit.
pub fn get_all_ancestors<S: ObjectStore>(
    store: &S,
    commit_id: &ObjectId,
) -> Result<HashSet<ObjectId>, HistoryError> {
    let mut ancestors = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(*commit_id);
    ancestors.insert(*commit_id);

    while let Some(current) = queue.pop_front() {
        let commit = get_commit(store, &current)?;
        for parent in &commit.parents {
            if ancestors.insert(*parent) {
                queue.push_back(*parent);
            }
        }
    }

    Ok(ancestors)
}

/// Finds the lowest common ancestors (merge bases) between two commits.
/// An ancestor C is a lowest common ancestor if no other common ancestor D is a descendant of C.
pub fn find_merge_bases<S: ObjectStore>(
    store: &S,
    commit_a: &ObjectId,
    commit_b: &ObjectId,
) -> Result<Vec<ObjectId>, HistoryError> {
    if commit_a == commit_b {
        return Ok(vec![*commit_a]);
    }

    if is_ancestor(store, commit_a, commit_b)? {
        return Ok(vec![*commit_a]);
    }

    if is_ancestor(store, commit_b, commit_a)? {
        return Ok(vec![*commit_b]);
    }

    let ancestors_a = get_all_ancestors(store, commit_a)?;
    let ancestors_b = get_all_ancestors(store, commit_b)?;

    // Common ancestors = intersection
    let common_ancestors: Vec<ObjectId> = ancestors_a.intersection(&ancestors_b).copied().collect();

    if common_ancestors.is_empty() {
        return Ok(vec![]);
    }

    // Filter to retain only maximal/lowest elements (not an ancestor of any other common ancestor)
    let mut lowest = Vec::new();
    for (i, cand) in common_ancestors.iter().enumerate() {
        let mut is_strictly_dominated = false;

        for (j, other) in common_ancestors.iter().enumerate() {
            if i != j && is_ancestor(store, cand, other)? {
                is_strictly_dominated = true;
                break;
            }
        }

        if !is_strictly_dominated && !lowest.contains(cand) {
            lowest.push(*cand);
        }
    }

    Ok(lowest)
}

/// Finds the single best merge base between two commits.
/// In case of criss-cross merges with multiple LCA candidates, selects the one with the latest timestamp.
pub fn find_best_merge_base<S: ObjectStore>(
    store: &S,
    commit_a: &ObjectId,
    commit_b: &ObjectId,
) -> Result<Option<ObjectId>, HistoryError> {
    let bases = find_merge_bases(store, commit_a, commit_b)?;
    if bases.is_empty() {
        return Ok(None);
    }
    if bases.len() == 1 {
        return Ok(Some(bases[0]));
    }

    // For multiple candidates (criss-cross history), pick the candidate with highest timestamp
    let mut best = bases[0];
    let mut best_ts = get_commit(store, &best)?.timestamp_nanos;

    for candidate in &bases[1..] {
        let ts = get_commit(store, candidate)?.timestamp_nanos;
        if ts > best_ts {
            best = *candidate;
            best_ts = ts;
        }
    }

    Ok(Some(best))
}

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::HashAlgorithm;
    use spine_format::serialize_commit;
    use spine_object_store::MemoryObjectStore;

    #[test]
    fn test_commit_graph_walk() {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // Commit 1 (root)
        let c1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![],
            "Author".into(),
            "a@test.com".into(),
            100,
            "c1".into(),
        );
        let c1_bytes = serialize_commit(&c1);
        let c1_id = store.write_object(ObjectType::Commit, &c1_bytes).unwrap();

        // Commit 2 (child of c1)
        let c2 = Commit::new_simple(
            ObjectId::ZERO,
            vec![c1_id],
            "Author".into(),
            "a@test.com".into(),
            200,
            "c2".into(),
        );
        let c2_bytes = serialize_commit(&c2);
        let c2_id = store.write_object(ObjectType::Commit, &c2_bytes).unwrap();

        let history = walk_history(&store, &c2_id, None).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, c2_id);
        assert_eq!(history[1].id, c1_id);

        assert!(is_ancestor(&store, &c1_id, &c2_id).unwrap());
        assert!(!is_ancestor(&store, &c2_id, &c1_id).unwrap());
    }

    #[test]
    fn test_divergent_branch_merge_base() {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // Root commit C0
        let c0 = Commit::new_simple(
            ObjectId::ZERO,
            vec![],
            "Dev".into(),
            "d@s.org".into(),
            10,
            "c0".into(),
        );
        let c0_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c0))
            .unwrap();

        // Branch A: C0 -> A1 -> A2
        let a1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![c0_id],
            "Dev".into(),
            "d@s.org".into(),
            20,
            "a1".into(),
        );
        let a1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&a1))
            .unwrap();
        let a2 = Commit::new_simple(
            ObjectId::ZERO,
            vec![a1_id],
            "Dev".into(),
            "d@s.org".into(),
            30,
            "a2".into(),
        );
        let a2_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&a2))
            .unwrap();

        // Branch B: C0 -> B1
        let b1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![c0_id],
            "Dev".into(),
            "d@s.org".into(),
            25,
            "b1".into(),
        );
        let b1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&b1))
            .unwrap();

        let base = find_best_merge_base(&store, &a2_id, &b1_id).unwrap();
        assert_eq!(base, Some(c0_id));
    }

    #[test]
    fn test_criss_cross_merge_base() {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // C0
        let c0 = Commit::new_simple(
            ObjectId::ZERO,
            vec![],
            "Dev".into(),
            "d@s.org".into(),
            10,
            "c0".into(),
        );
        let c0_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c0))
            .unwrap();

        // A1, B1
        let a1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![c0_id],
            "Dev".into(),
            "d@s.org".into(),
            20,
            "a1".into(),
        );
        let a1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&a1))
            .unwrap();
        let b1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![c0_id],
            "Dev".into(),
            "d@s.org".into(),
            25,
            "b1".into(),
        );
        let b1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&b1))
            .unwrap();

        // Criss-cross: M1 merges A1 and B1; M2 merges B1 and A1
        let m1 = Commit::new_simple(
            ObjectId::ZERO,
            vec![a1_id, b1_id],
            "Dev".into(),
            "d@s.org".into(),
            40,
            "m1".into(),
        );
        let m1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&m1))
            .unwrap();

        let m2 = Commit::new_simple(
            ObjectId::ZERO,
            vec![b1_id, a1_id],
            "Dev".into(),
            "d@s.org".into(),
            50,
            "m2".into(),
        );
        let m2_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&m2))
            .unwrap();

        let bases = find_merge_bases(&store, &m1_id, &m2_id).unwrap();
        assert!(bases.contains(&a1_id));
        assert!(bases.contains(&b1_id));

        let best = find_best_merge_base(&store, &m1_id, &m2_id).unwrap();
        // b1 timestamp (25) > a1 timestamp (20)
        assert_eq!(best, Some(b1_id));
    }
}
