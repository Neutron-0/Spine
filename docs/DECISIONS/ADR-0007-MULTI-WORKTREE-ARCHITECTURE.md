# ADR-0007: Linked Multi-Worktree Architecture and Shared Object Store

## Status
Accepted

## Context
Modern software development workflows frequently require context switching between feature branches, hotfixes, and pull requests without stashing or disturbing ongoing builds. Developers need multiple working trees attached to a single Spine repository.

A naive approach of duplicating repositories wastes disk space and requires continuous inter-repo pushes. Spine requires a first-class linked worktree engine sharing storage, configuration, and remote tracking state while isolating working trees, staging indexes, and branch heads.

## Decision

1. **Repository Layout and Linkage**:
   - The primary repository retains `.spine/` as a directory containing the authoritative object database (`objects/`, `pack/`, `packs.catalog`), shared references (`refs/heads/`, `refs/tags/`, `refs/remotes/`), configuration (`spine.toml`), and operation journal.
   - Linked worktrees contain a lightweight `.spine` text file containing a single pointer directive:
     ```
     gitdir: <absolute_or_canonical_path_to_primary_spine_dir>/worktrees/<worktree_id>
     ```
   - Inside `<primary_spine_dir>/worktrees/<worktree_id>/`, Spine stores worktree-specific state:
     - `HEAD`: Symref to checked-out branch or direct commit hash (detached).
     - `index`: Binary staging index dedicated to this worktree.
     - `worktree.json`: Metadata tracking worktree id, canonical worktree path, branch, lock status, lock reason, and timestamp.
     - `journal`: Worktree-specific transaction log linked to the primary journal.

2. **Strict Branch Ownership Locking**:
   - Multiple worktrees may not check out the same local branch simultaneously.
   - `spine worktree add <path> <branch>` queries `get_checked_out_branches()` across the primary repo and all registered worktrees. If `<branch>` is currently checked out elsewhere, the operation fails immediately with an informative error.
   - Detached HEAD checkouts are permitted across multiple worktrees.

3. **Storage and Configuration Sharing**:
   - Every CLI execution resolves repository context via `spine_worktree::resolve_worktree_context()`.
   - The composite object store (`CompositeObjectStore`) is always opened against the primary repository root (`primary_spine_dir`). All object writes (blobs, trees, commits, tags) flow into shared loose storage or packfiles.
   - Reference updates for branches and tags update `<primary_spine_dir>/refs/`, ensuring updates in one worktree are immediately visible throughout the repository.

4. **Lifecycle and Garbage Collection**:
   - Worktrees can be added (`spine worktree add`), listed (`spine worktree list`), removed (`spine worktree remove`), and pruned (`spine worktree prune`).
   - Garbage collection (`spine gc`) traverses the `HEAD` and `index` roots of ALL registered linked worktrees in addition to primary refs. Objects referenced solely by a linked worktree or its staging index are preserved during pruning.

## Consequences

- Near-instant worktree creation without copying object history.
- Zero risk of silent branch desynchronization or race conditions.
- Reliable disk cleanup via prune operations.
