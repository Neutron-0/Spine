# Spine Operation Journal & Workspace Recovery

## Overview
Spine features an append-only, transactional operation journal located in `<primary_spine_dir>/journal/` and `<linked_spine_dir>/journal/`. Every state-mutating command records a two-phase transaction ensuring crash recovery and multi-step undo/redo.

## Transaction Lifecycle

```
[In-flight State Mutation]
         │
         ▼
┌──────────────────┐
│ prepare_transaction │ ──> Writes snapshot to journal/wal.log (State = Preparing)
└──────────────────┘
         │
         ▼
[Perform Disk Edits] (Tree writes, ref updates, index changes)
         │
         ▼
┌──────────────────┐
│ commit_transaction  │ ──> Writes completion marker (State = Committed)
└──────────────────┘
```

## Batch 06 Expanded Operations

Batch 06 introduced first-class journal entries for advanced history and worktree commands:

| OperationKind | Description | State Captured |
|---|---|---|
| `Reset` | Soft, mixed, or hard reset | Preflight branch tip, index hash, worktree tree |
| `Restore` | File restoration from tree/index | Modified paths and pre-restoration content |
| `Revert` | Inverted commit creation | Pre-revert HEAD and index |
| `CherryPick` | Patch replay onto HEAD | Target commit and pre-cherry-pick state |
| `Rebase` | Topological commit sequence replay | Original branch tip, rebase-apply state |
| `StashPush` | Stash stack creation | Working directory and index changes |
| `StashApply` | Stash reinstatement | Stash index and modified files |
| `StashDrop` | Stash entry deletion | Dropped stash commit ID |
| `WorktreeAdd` | Linked worktree registration | Worktree path, target branch, and ID |
| `WorktreeRemove` | Linked worktree cleanup | Removed worktree path and metadata |
| `Bisect` | Binary search step | Checked out candidate and bisect boundaries |

## Crash Recovery on `fsck`

If a process terminates mid-operation (power loss, SIGKILL), `spine fsck` analyzes the journal:
- Uncommitted transactions in `Preparing` state are detected as interrupted.
- If snapshot data is present, `spine fsck` rolls back partial reference and index updates, returning the workspace to its pre-operation consistent state.
