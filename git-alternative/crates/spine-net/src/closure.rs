//! Push object-closure validation: validates that all reachable commits,
//! trees, subtrees, and blobs are fully present and intact in the object store
//! before reference publication.

use spine_core::{ObjectId, ObjectType};
use spine_format::{deserialize_commit, deserialize_tree};
use spine_object_store::{CompositeObjectStore, ObjectStore, ObjectStoreError};
use std::collections::HashSet;
use thiserror::Error;

#[derive(Error, Debug, PartialEq, Eq)]
pub enum ClosureError {
    #[error("Object closure violation: missing object {0} (type {1:?})")]
    MissingObject(ObjectId, ObjectType),
    #[error("Object corruption or deserialization failure for {0}: {1}")]
    CorruptObject(ObjectId, String),
    #[error("Storage error: {0}")]
    Storage(String),
}

/// Recursively verifies that all objects in the transitive graph rooted at `root_oid`
/// (commits, parent commits, trees, subtrees, and blobs) exist in `store` and are readable.
///
/// Stops descending past commits contained in `known_boundary` to avoid re-traversing
/// entire historical graphs on incremental pushes.
pub fn validate_push_object_closure(
    store: &CompositeObjectStore,
    root_oid: &ObjectId,
    known_boundary: &HashSet<ObjectId>,
) -> Result<usize, ClosureError> {
    let mut visited: HashSet<ObjectId> = HashSet::new();
    let mut commit_queue: Vec<ObjectId> = vec![*root_oid];
    let mut verified_count = 0;

    // 1. Traverse commits
    while let Some(commit_oid) = commit_queue.pop() {
        if known_boundary.contains(&commit_oid) || visited.contains(&commit_oid) {
            continue;
        }

        let (obj_type, data) = match store.read_object(&commit_oid) {
            Ok(pair) => pair,
            Err(ObjectStoreError::NotFound(oid)) => {
                return Err(ClosureError::MissingObject(oid, ObjectType::Commit));
            }
            Err(e) => return Err(ClosureError::Storage(e.to_string())),
        };

        if obj_type != ObjectType::Commit {
            if obj_type == ObjectType::Tree {
                validate_tree_closure(store, &commit_oid, &mut visited, &mut verified_count)?;
                continue;
            }
            return Err(ClosureError::CorruptObject(
                commit_oid,
                format!("Expected Commit object, got {:?}", obj_type),
            ));
        }

        let commit = deserialize_commit(&data)
            .map_err(|e| ClosureError::CorruptObject(commit_oid, e.to_string()))?;

        visited.insert(commit_oid);
        verified_count += 1;

        // Traverse root tree of this commit
        validate_tree_closure(store, &commit.root_tree, &mut visited, &mut verified_count)?;

        // Enqueue parent commits
        for parent_oid in commit.parents {
            if !known_boundary.contains(&parent_oid) && !visited.contains(&parent_oid) {
                commit_queue.push(parent_oid);
            }
        }
    }

    Ok(verified_count)
}

/// Recursively traverses and validates a tree and all its descendants (subtrees and blobs).
fn validate_tree_closure(
    store: &CompositeObjectStore,
    tree_oid: &ObjectId,
    visited: &mut HashSet<ObjectId>,
    verified_count: &mut usize,
) -> Result<(), ClosureError> {
    if visited.contains(tree_oid) {
        return Ok(());
    }

    let (obj_type, data) = match store.read_object(tree_oid) {
        Ok(pair) => pair,
        Err(ObjectStoreError::NotFound(oid)) => {
            return Err(ClosureError::MissingObject(oid, ObjectType::Tree));
        }
        Err(e) => return Err(ClosureError::Storage(e.to_string())),
    };

    if obj_type != ObjectType::Tree {
        return Err(ClosureError::CorruptObject(
            *tree_oid,
            format!("Expected Tree object, got {:?}", obj_type),
        ));
    }

    visited.insert(*tree_oid);
    *verified_count += 1;

    let entries = deserialize_tree(&data)
        .map_err(|e| ClosureError::CorruptObject(*tree_oid, e.to_string()))?;

    for entry in entries {
        if visited.contains(&entry.object_id) {
            continue;
        }

        match entry.mode {
            spine_core::FileMode::Directory => {
                validate_tree_closure(store, &entry.object_id, visited, verified_count)?;
            }
            spine_core::FileMode::Regular | spine_core::FileMode::Executable => {
                // Verify blob presence
                let (blob_type, _) = match store.read_object(&entry.object_id) {
                    Ok(pair) => pair,
                    Err(ObjectStoreError::NotFound(oid)) => {
                        return Err(ClosureError::MissingObject(oid, ObjectType::Blob));
                    }
                    Err(e) => return Err(ClosureError::Storage(e.to_string())),
                };

                if blob_type != ObjectType::Blob {
                    return Err(ClosureError::CorruptObject(
                        entry.object_id,
                        format!("Expected Blob object, got {:?}", blob_type),
                    ));
                }

                visited.insert(entry.object_id);
                *verified_count += 1;
            }
            spine_core::FileMode::Symlink => {
                // Verify symlink object
                if !store.has_object(&entry.object_id) {
                    return Err(ClosureError::MissingObject(
                        entry.object_id,
                        ObjectType::Blob,
                    ));
                }
                visited.insert(entry.object_id);
                *verified_count += 1;
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use spine_core::{Commit, FileMode, HashAlgorithm, TreeEntry};
    use spine_format::{serialize_commit, serialize_tree};
    use tempfile::tempdir;

    #[test]
    fn test_closure_validation_complete_graph() {
        let temp = tempdir().unwrap();
        let store = CompositeObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        // 1. Write blobs
        let b1 = store
            .write_object(ObjectType::Blob, b"file 1 content")
            .unwrap();
        let b2 = store
            .write_object(ObjectType::Blob, b"file 2 content")
            .unwrap();

        // 2. Write subtree
        let subtree_entries = vec![TreeEntry {
            name: "subfile.txt".to_string(),
            mode: FileMode::Regular,
            object_id: b2,
        }];
        let subtree_bytes = serialize_tree(&subtree_entries);
        let subtree_oid = store
            .write_object(ObjectType::Tree, &subtree_bytes)
            .unwrap();

        // 3. Write root tree
        let root_entries = vec![
            TreeEntry {
                name: "file1.txt".to_string(),
                mode: FileMode::Regular,
                object_id: b1,
            },
            TreeEntry {
                name: "dir".to_string(),
                mode: FileMode::Directory,
                object_id: subtree_oid,
            },
        ];
        let root_tree_bytes = serialize_tree(&root_entries);
        let root_tree_oid = store
            .write_object(ObjectType::Tree, &root_tree_bytes)
            .unwrap();

        // 4. Write commit
        let commit = Commit::new_simple(
            root_tree_oid,
            vec![],
            "Author".to_string(),
            "author@example.com".to_string(),
            100,
            "Initial commit".to_string(),
        );
        let commit_bytes = serialize_commit(&commit);
        let commit_oid = store
            .write_object(ObjectType::Commit, &commit_bytes)
            .unwrap();

        // Validate closure: all 4 objects should be verified
        let boundary = HashSet::new();
        let count = validate_push_object_closure(&store, &commit_oid, &boundary).unwrap();
        assert_eq!(count, 5); // commit, root tree, subtree, b1, b2
    }

    #[test]
    fn test_closure_validation_detects_missing_blob() {
        let temp = tempdir().unwrap();
        let store = CompositeObjectStore::new(temp.path(), HashAlgorithm::Blake3, 3).unwrap();

        let missing_blob = ObjectId::from_bytes([0xAA; 32]);

        let root_entries = vec![TreeEntry {
            name: "missing.txt".to_string(),
            mode: FileMode::Regular,
            object_id: missing_blob,
        }];
        let root_tree_bytes = serialize_tree(&root_entries);
        let root_tree_oid = store
            .write_object(ObjectType::Tree, &root_tree_bytes)
            .unwrap();

        let commit = Commit::new_simple(
            root_tree_oid,
            vec![],
            "Author".to_string(),
            "author@example.com".to_string(),
            100,
            "Commit with missing blob".to_string(),
        );
        let commit_bytes = serialize_commit(&commit);
        let commit_oid = store
            .write_object(ObjectType::Commit, &commit_bytes)
            .unwrap();

        let boundary = HashSet::new();
        let err = validate_push_object_closure(&store, &commit_oid, &boundary).unwrap_err();
        match err {
            ClosureError::MissingObject(oid, obj_type) => {
                assert_eq!(oid, missing_blob);
                assert_eq!(obj_type, ObjectType::Blob);
            }
            other => panic!("Expected MissingObject, got {:?}", other),
        }
    }
}
