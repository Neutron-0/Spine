# Spine Revert and Cherry-Pick Engine

## Overview
`spine cherry-pick` and `spine revert` apply or invert specific commit changesets across branches.

## `spine cherry-pick`

Applies the changes introduced by an existing commit onto current HEAD.

```bash
spine cherry-pick <commit-ish>
```

### Execution Flow
1. **Target Analysis**:
   - Resolves target revision (commit hash, branch, or tag).
   - If target commit is a merge commit (multiple parents), `--mainline <n>` (1-indexed) must be specified.
2. **Three-Way Merge Replay**:
   - Computes diff between parent commit (or mainline parent) and target commit.
   - Merges diff onto current HEAD root tree using `merge_3way_trees`.
3. **Commit Creation**:
   - Preserves original author name, email, and timestamp.
   - Records current user as committer with current timestamp.
   - Retains original commit message.
4. **Conflict Handling**:
   - If conflicts arise, Spine saves `.spine/CHERRY_PICK_STATE` and `.spine/MERGE_STATE`.
   - Supports `--continue` (after conflict resolution), `--skip`, and `--abort`.

## `spine revert`

Reverts an existing commit by applying its inverted changeset.

```bash
spine revert [--mainline <n>] <commit-ish>
```

### Execution Flow
1. Resolves target revision.
2. Inverts diff (swapping target tree and parent tree).
3. Merges inverted diff onto current HEAD.
4. Automatically generates commit message:
   ```
   Revert "<original message>"
   
   This reverts commit <full-oid>.
   ```
5. In case of conflicts, saves `.spine/REVERT_STATE` with `--continue` and `--abort` support.
