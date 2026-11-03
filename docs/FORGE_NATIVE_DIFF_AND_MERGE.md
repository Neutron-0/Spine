# Spine Forge: Native VCS Diff, Mergeability, and Atomic Merge Engine

## 1. Zero Git Dependency Principle

Spine Forge performs all version-control operations natively in Rust. It does **not** invoke the `git` binary, `libgit2`, or `gitoxide` at runtime. All diff calculations, tree flattenings, merge-base lookups, three-way tree reconciliations, object writings, and atomic ref updates are powered by:
- `spine-core`: Object model, `ObjectId`, `FileMode`, and hashing.
- `spine-object-store`: Content-addressed storage and loose/pack object retrieval.
- `spine-diff`: Myers diff algorithm and unified diff formatting.
- `spine-merge`: 3-way tree and content merge engine with conflict detection.
- `spine-history`: Commit DAG traversal and merge base discovery.

---

## 2. Tree Flattening and Recursive Hierarchy

Spine stores directory trees as hierarchical `Tree` objects (`spine_core::Tree`), where each entry may be a blob or a sub-tree. The diff and merge engines require deterministic flat mappings of paths to modes and object IDs.

Implemented in `github-alternative/backend/apps/spine-api/src/storage/vcs.rs`:

```rust
pub fn flatten_tree(
    store: &FilesystemObjectStore,
    tree_id: ObjectId,
    prefix: &str,
    out: &mut BTreeMap<String, (FileMode, ObjectId)>,
) -> Result<(), SpineApiError> {
    let tree = store.read_tree(tree_id)?;
    for entry in tree.entries() {
        let path = if prefix.is_empty() {
            entry.name().to_string()
        } else {
            format!("{}/{}", prefix, entry.name())
        };

        if entry.mode() == FileMode::Tree {
            flatten_tree(store, entry.id(), &path, out)?;
        } else {
            out.insert(path, (entry.mode(), entry.id()));
        }
    }
    Ok(())
}
```

Conversely, writing a merged flat map back to a hierarchical tree uses `write_tree_recursive`, grouping entries by path component and creating `Tree` objects from leaf to root.

---

## 3. Native Unified Diff Generation

Unified diffs for PRs and commits are generated via `spine_diff::format_unified_diff(path, old_text, new_text)`.

```rust
let old_bytes = store.read_blob(old_id).unwrap_or_default();
let new_bytes = store.read_blob(new_id).unwrap_or_default();
let old_str = String::from_utf8_lossy(&old_bytes);
let new_str = String::from_utf8_lossy(&new_bytes);

let patch = spine_diff::format_unified_diff(&path, &old_str, &new_str);
```

For each modified file, additions and deletions are computed by counting lines prefixed with `+` and `-`.

---

## 4. Mergeability Analysis

Before merging, `check_mergeability` inspects the commit DAG:
1. Resolves `base_commit_id` and `head_commit_id`.
2. Computes the merge base (common ancestor). If `base_commit_id == head_commit_id`, it is trivial.
3. If `base_commit_id == merge_base`, the change is a **clean fast-forward**.
4. Otherwise, runs `spine_merge::merge_3way_trees`:
   ```rust
   let merge_result = spine_merge::merge_3way_trees(
       &base_entries,
       &head_entries,
       &ancestor_entries,
       &store,
   )?;
   ```
5. If `merge_result.has_conflicts` is true, the PR is marked `mergeable: false` and lists conflicting file paths. Otherwise, `mergeable: true`.

---

## 5. Atomic Three-Way Merge Execution

When a user merges a pull request:
1. **Branch Protection Verification:** `permissions.check_merge_allowed(repo, base_branch, pr, user_id)` verifies required reviews, lack of requested changes, no unaddressed inline comments, and absence of self-approval.
2. **Commit Creation:**
   - A new native `Commit` object is assembled with **two parents**: `[base_commit_id, head_commit_id]`.
   - The tree ID is the root `ObjectId` resulting from `write_tree_recursive`.
   - Committer and author fields are attributed to the authenticated user.
   - The commit object is written to the repository's `.spine/objects` store.
3. **Atomic Ref Update:**
   - The new commit ID is written atomically to `.spine/refs/heads/<base_branch>`.
4. **Post-Merge Hooks:**
   - Pull request status transitions to `MERGED`.
   - Referenced issues (`Fixes #123`) are automatically closed.
   - Notifications and activity feed records are emitted.
