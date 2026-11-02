# Spine Rebase Engine

## Overview
`spine rebase` enables linear history reorganization by replaying commits from an upstream branch onto a new base commit.

## Command Syntax

```bash
spine rebase <upstream>
spine rebase --onto <newbase> <upstream>
spine rebase --continue
spine rebase --skip
spine rebase --abort
```

## Architecture and Replay Mechanics

### 1. Merge Base Discovery & Replay Sequence
- Spine computes the unique merge base between `<upstream>` and current `HEAD` via `spine_history::find_unique_merge_base`.
- It collects commits in topological Kahn sort order between merge base and `HEAD` via `spine_history::find_commits_between`.

### 2. State Persistence (`.spine/rebase-apply/`)
- Rebase state is saved in JSON format containing:
  - `onto`: Target commit OID.
  - `orig_head`: HEAD before rebase started.
  - `branch_name`: Name of branch being rebased.
  - `remaining_commits`: Unapplied commit queue.
  - `current_commit`: Currently active commit.
  - `old_to_new`: Mapping of historical commit hashes to rebased commit hashes.

### 3. Execution Pipeline
1. Switches working-tree and staging index to `<onto>` root tree.
2. For each commit in `remaining_commits`:
   - Computes 3-way tree merge between commit's parent, `<onto>`, and `commit`.
   - If clean: writes merged tree, creates new commit with parent `<onto>`, updates `old_to_new`, updates `<onto> = new_commit`, pops commit from queue.
   - If conflicted: writes conflict markers, writes `.spine/MERGE_STATE`, saves `.spine/rebase-apply/state.json`, and exits with error code.
3. Upon completing all commits:
   - Updates target branch ref in `<primary_spine_dir>/refs/heads/<branch>` to the final rebased commit.
   - Clears `.spine/rebase-apply/`.

### 4. Conflict Resolution & Abort
- `spine rebase --continue`: Validates `MERGE_STATE` is fully resolved, commits current index, and resumes remaining commits.
- `spine rebase --skip`: Skips current commit without applying its diff.
- `spine rebase --abort`: Restores original branch tip, resets working-tree and index to `orig_head`, and removes rebase state directory.
