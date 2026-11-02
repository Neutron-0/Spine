# ADR-0006: Advanced History Operations & Replay Architecture

## Status
Accepted

## Context
As SPINE matures from an append-only snapshot engine to a complete daily-driver distributed version-control system, developers require flexible history manipulation tools: `reset`, `restore`, `revert`, `cherry-pick`, and `rebase`. 

Prior systems (e.g. early Git) relied on shell-script wrappers invoking low-level plumbing commands, creating fragility, race conditions during interruptions, and potential working tree corruption. Spine requires an industrial, crash-resilient history replay architecture implemented entirely in native Rust, backed by journaled transactions and deterministic state persistence.

## Decision

1. **Rebase Engine State Machine**:
   - Rebase operations are modeled as state machines persisted in `.spine/rebase-apply/state.json`.
   - The state record includes:
     - `onto`: The base commit OID onto which commits are replayed.
     - `orig_head`: Original HEAD commit prior to rebase initiation.
     - `branch_name`: The active branch name being rebased.
     - `remaining_commits`: Ordered list of commit OIDs remaining in the replay queue (topologically sorted from merge base to HEAD).
     - `current_commit`: Commit currently being evaluated or paused for conflict resolution.
     - `old_to_new`: Mapping of historical commit OIDs to newly generated rebased commit OIDs.
   - Rebase automatically executes a safe working-tree switch to `onto` before sequentially applying 3-way tree merges.
   - When conflicts occur, Spine creates a `MERGE_STATE` record alongside `rebase-apply/state.json` and halts cleanly.
   - `--continue` verifies conflict resolution, commits the index, records the `old_to_new` mapping, and resumes replay.
   - `--skip` discards the current commit and continues replay of remaining commits.
   - `--abort` restores working-tree, staging index, and branch ref to `orig_head` and removes `.spine/rebase-apply/`.

2. **Cherry-Pick and Revert**:
   - Cherry-pick extracts the diff between `commit` and its first parent, applying it via 3-way tree merge onto current HEAD. Author identity, author email, and author timestamp are preserved; committer identity and timestamp are refreshed.
   - Revert computes an inverted 3-way diff (swapping parent and target commit), producing a new commit that undoes the targeted revision while preserving history linearly.
   - Both commands enforce explicit `--mainline <n>` specifications when operating on multi-parent merge commits.
   - State persistence (`.spine/CHERRY_PICK_STATE` and `.spine/REVERT_STATE`) enables robust `--continue` and `--abort` workflows.

3. **Reset and Restore**:
   - `spine reset <rev>` defaults to mixed reset (`--mixed`), updating branch tip and staging index while preserving working-tree modifications.
   - `spine reset --soft <rev>` moves HEAD and branch reference only, preserving both index and working tree.
   - `spine reset --hard <rev>` resets HEAD, index, and working tree to the target commit after verifying safety against uncommitted data loss and recording a full preflight transaction snapshot in `spine-journal`.
   - Path-specific reset (`spine reset <rev> <paths>...`) selectively copies object references from target commit tree into the staging index.
   - `spine restore` safely recovers files in the working directory and/or staging index (`--staged`, `--worktree`, `--source <rev>`), preventing symlink traversal escapes.

## Consequences

- Zero data loss during interrupted rebases or cherry-picks.
- Complete auditability through operation journaling.
- Seamless compatibility with garbage collection: in-flight replay commits and original heads are registered as GC roots, preventing accidental pruning.
