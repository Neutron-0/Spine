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
        let c1 = Commit {
            root_tree: ObjectId::ZERO,
            parents: vec![],
            author_name: "Author".into(),
            author_email: "a@test.com".into(),
            timestamp_nanos: 100,
            message: "c1".into(),
        };
        let c1_bytes = serialize_commit(&c1);
        let c1_id = store.write_object(ObjectType::Commit, &c1_bytes).unwrap();

        // Commit 2 (child of c1)
        let c2 = Commit {
            root_tree: ObjectId::ZERO,
            parents: vec![c1_id],
            author_name: "Author".into(),
            author_email: "a@test.com".into(),
            timestamp_nanos: 200,
            message: "c2".into(),
        };
        let c2_bytes = serialize_commit(&c2);
        let c2_id = store.write_object(ObjectType::Commit, &c2_bytes).unwrap();

        let history = walk_history(&store, &c2_id, None).unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, c2_id);
        assert_eq!(history[1].id, c1_id);
    }
}
