//! # spine-history
//!
//! Commit graph navigation, ancestor discovery, and topological history traversal.

pub mod bisect;
pub mod blame;

pub use bisect::BisectState;
pub use blame::{compute_blame, find_path_in_tree, BlameError, BlameLine, BlameResult};

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
    #[error("Ambiguous merge bases detected ({0} candidates: {1}); criss-cross history cannot be resolved automatically without virtual base synthesis")]
    AmbiguousMergeBases(usize, String),
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

/// Finds the single unambiguous merge base between two commits.
/// Fails safely with `HistoryError::AmbiguousMergeBases` if criss-cross history presents multiple lowest common ancestors.
pub fn find_unique_merge_base<S: ObjectStore>(
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

    let id_list = bases
        .iter()
        .map(|b| b.to_hex())
        .collect::<Vec<_>>()
        .join(", ");
    Err(HistoryError::AmbiguousMergeBases(bases.len(), id_list))
}

/// Backwards-compatible alias for `find_unique_merge_base`.
/// Rejects ambiguous criss-cross histories rather than arbitrarily picking timestamps.
pub fn find_best_merge_base<S: ObjectStore>(
    store: &S,
    commit_a: &ObjectId,
    commit_b: &ObjectId,
) -> Result<Option<ObjectId>, HistoryError> {
    find_unique_merge_base(store, commit_a, commit_b)
}

/// Returns the list of commits reachable from `tip` but not reachable from `base`
/// (the range `base..tip`), topologically ordered from oldest (child of base) to newest (`tip`).
pub fn find_commits_between<S: ObjectStore>(
    store: &S,
    base: &ObjectId,
    tip: &ObjectId,
) -> Result<Vec<ObjectId>, HistoryError> {
    if base == tip {
        return Ok(Vec::new());
    }

    let base_ancestors = get_all_ancestors(store, base)?;
    if base_ancestors.contains(tip) {
        // tip is an ancestor of base; range is empty
        return Ok(Vec::new());
    }

    // Collect all commits reachable from tip that are not in base_ancestors
    let mut commits_in_range = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(*tip);
    commits_in_range.insert(*tip);

    while let Some(curr) = queue.pop_front() {
        let commit = get_commit(store, &curr)?;
        for parent in &commit.parents {
            if !base_ancestors.contains(parent) && commits_in_range.insert(*parent) {
                queue.push_back(*parent);
            }
        }
    }

    // Topologically sort commits_in_range: parents before children (oldest to newest)
    let mut in_degree: std::collections::HashMap<ObjectId, usize> =
        std::collections::HashMap::new();
    let mut children_map: std::collections::HashMap<ObjectId, Vec<ObjectId>> =
        std::collections::HashMap::new();

    for id in &commits_in_range {
        in_degree.entry(*id).or_insert(0);
        let commit = get_commit(store, id)?;
        for parent in &commit.parents {
            if commits_in_range.contains(parent) {
                *in_degree.entry(*id).or_insert(0) += 1;
                children_map.entry(*parent).or_default().push(*id);
            }
        }
    }

    let mut ready: VecDeque<ObjectId> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(&id, _)| id)
        .collect();

    let mut ordered = Vec::with_capacity(commits_in_range.len());
    while let Some(node) = ready.pop_front() {
        ordered.push(node);
        if let Some(children) = children_map.get(&node) {
            for child in children {
                if let Some(deg) = in_degree.get_mut(child) {
                    *deg -= 1;
                    if *deg == 0 {
                        ready.push_back(*child);
                    }
                }
            }
        }
    }

    Ok(ordered)
}

/// Finds an optimal commit midpoint for binary search bisecting.
///
/// Given one or more `good_commits` (known good), a `bad_commit` (known bad),
/// and an optional set of `skipped` commits:
/// Returns a candidate commit from `(ancestors(bad) \ ancestors(good)) \ skipped`
/// whose ancestor count inside the candidate set is as close to half of the candidate set as possible.
pub fn find_bisect_midpoint<S: ObjectStore>(
    store: &S,
    good_commits: &[ObjectId],
    bad_commit: &ObjectId,
    skipped: &HashSet<ObjectId>,
) -> Result<Option<ObjectId>, HistoryError> {
    let mut good_ancestors = HashSet::new();
    for good in good_commits {
        good_ancestors.extend(get_all_ancestors(store, good)?);
    }

    if good_ancestors.contains(bad_commit) {
        return Ok(None);
    }

    let bad_ancestors = get_all_ancestors(store, bad_commit)?;
    let candidates: HashSet<ObjectId> =
        bad_ancestors.difference(&good_ancestors).copied().collect();

    if candidates.is_empty() {
        return Ok(None);
    }

    let eligible: Vec<ObjectId> = candidates
        .iter()
        .copied()
        .filter(|id| !skipped.contains(id) && id != bad_commit)
        .collect();

    if eligible.is_empty() {
        if !skipped.contains(bad_commit) {
            return Ok(Some(*bad_commit));
        }
        return Ok(None);
    }

    let total_candidates = candidates.len();
    let target = total_candidates / 2;

    let mut best_commit = eligible[0];
    let mut best_diff = usize::MAX;

    for &cand in &eligible {
        let cand_ancestors = get_all_ancestors(store, &cand)?;
        let count_in_candidates = cand_ancestors.intersection(&candidates).count();
        let diff = count_in_candidates.abs_diff(target);

        if diff < best_diff {
            best_diff = diff;
            best_commit = cand;
        }
    }

    Ok(Some(best_commit))
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
        assert_eq!(bases.len(), 2);
        assert!(bases.contains(&a1_id));
        assert!(bases.contains(&b1_id));

        // Attempting to pick a single best base without virtual synthesis must fail safely with AmbiguousMergeBases
        let res = find_best_merge_base(&store, &m1_id, &m2_id);
        match res {
            Err(HistoryError::AmbiguousMergeBases(count, ids)) => {
                assert_eq!(count, 2);
                assert!(ids.contains(&a1_id.to_hex()));
                assert!(ids.contains(&b1_id.to_hex()));
            }
            other => panic!("Expected AmbiguousMergeBases, got {:?}", other),
        }
    }

    #[test]
    fn test_find_commits_between_and_bisect() {
        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // Linear chain: C0 -> C1 -> C2 -> C3 -> C4
        let mut prev = None;
        let mut ids = Vec::new();
        for i in 0..5 {
            let parents = if let Some(p) = prev { vec![p] } else { vec![] };
            let c = Commit::new_simple(
                ObjectId::ZERO,
                parents,
                "Dev".into(),
                "d@s.org".into(),
                i * 100,
                format!("commit {}", i),
            );
            let cid = store
                .write_object(ObjectType::Commit, &serialize_commit(&c))
                .unwrap();
            ids.push(cid);
            prev = Some(cid);
        }

        // Commits between C1 and C4 should be [C2, C3, C4] in topological order
        let between = find_commits_between(&store, &ids[1], &ids[4]).unwrap();
        assert_eq!(between, vec![ids[2], ids[3], ids[4]]);

        // Empty range when base == tip
        let empty = find_commits_between(&store, &ids[2], &ids[2]).unwrap();
        assert!(empty.is_empty());

        // Bisect midpoint with good=C0, bad=C4
        // candidates: C1, C2, C3, C4 (4 total)
        // midpoint should divide candidates evenly (e.g. C2)
        let skipped = HashSet::new();
        let mid = find_bisect_midpoint(&store, &[ids[0]], &ids[4], &skipped).unwrap();
        assert!(mid == Some(ids[2]) || mid == Some(ids[1]));
    }

    #[test]
    fn test_blame_attribution() {
        use spine_core::{FileMode, TreeEntry};
        use spine_format::serialize_tree;

        let store = MemoryObjectStore::new(HashAlgorithm::Blake3);

        // Commit 1 creates "file.txt" with 2 lines: "line 1", "line 2"
        let b1 = store
            .write_object(ObjectType::Blob, b"line 1\nline 2\n")
            .unwrap();
        let t1 = store
            .write_object(
                ObjectType::Tree,
                &serialize_tree(&[TreeEntry {
                    mode: FileMode::Regular,
                    name: "file.txt".into(),
                    object_id: b1,
                }]),
            )
            .unwrap();
        let c1 = Commit::new_simple(
            t1,
            vec![],
            "Alice".into(),
            "alice@spine.org".into(),
            1000,
            "Initial commit".into(),
        );
        let c1_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c1))
            .unwrap();

        // Commit 2 modifies "file.txt": "line 1", "line 2 changed by bob", "line 3 added by bob"
        let b2 = store
            .write_object(
                ObjectType::Blob,
                b"line 1\nline 2 changed by bob\nline 3 added by bob\n",
            )
            .unwrap();
        let t2 = store
            .write_object(
                ObjectType::Tree,
                &serialize_tree(&[TreeEntry {
                    mode: FileMode::Regular,
                    name: "file.txt".into(),
                    object_id: b2,
                }]),
            )
            .unwrap();
        let c2 = Commit::new_simple(
            t2,
            vec![c1_id],
            "Bob".into(),
            "bob@spine.org".into(),
            2000,
            "Bob edits".into(),
        );
        let c2_id = store
            .write_object(ObjectType::Commit, &serialize_commit(&c2))
            .unwrap();

        let blame = compute_blame(&store, &c2_id, "file.txt").unwrap();
        assert_eq!(blame.lines.len(), 3);

        // Line 1: should be attributed to Alice (c1_id)
        assert_eq!(blame.lines[0].author_name, "Alice");
        assert_eq!(blame.lines[0].commit_id, c1_id);
        assert_eq!(blame.lines[0].content, "line 1");

        // Line 2: modified by Bob (c2_id)
        assert_eq!(blame.lines[1].author_name, "Bob");
        assert_eq!(blame.lines[1].commit_id, c2_id);
        assert_eq!(blame.lines[1].content, "line 2 changed by bob");

        // Line 3: added by Bob (c2_id)
        assert_eq!(blame.lines[2].author_name, "Bob");
        assert_eq!(blame.lines[2].commit_id, c2_id);
        assert_eq!(blame.lines[2].content, "line 3 added by bob");
    }
}
