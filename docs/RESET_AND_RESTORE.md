# Spine Reset and Restore Engine

## Overview
`spine reset` and `spine restore` provide fine-grained workspace and history recovery capabilities.

## `spine reset`

`spine reset` resets current HEAD to the specified revision, optionally modifying the index and working tree.

### Modes

1. **Mixed Reset (`spine reset [--mixed] <rev>`) [Default]**:
   - Updates the current branch ref and HEAD to `<rev>`.
   - Resets the staging index to match the target commit's root tree.
   - Leaves working tree files untouched.
   - Any modifications previously staged become unstaged modifications.

2. **Soft Reset (`spine reset --soft <rev>`)**:
   - Updates the current branch ref and HEAD to `<rev>`.
   - Leaves the staging index completely untouched.
   - Leaves working tree files untouched.
   - Useful for consolidating multiple commits into a single staged changeset.

3. **Hard Reset (`spine reset --hard <rev>`)**:
   - Updates the current branch ref and HEAD to `<rev>`.
   - Resets the staging index to match the target commit's root tree.
   - Resets the working tree to match the target commit's root tree (overwriting modified tracked files and removing untracked files added by newer commits).
   - **Preflight Safety**: Spine takes a preflight repository snapshot recorded in `spine-journal`, allowing full recovery via `spine undo` in case of accidental data reset.

### Path-Specific Reset

`spine reset <rev> <paths>...`
- Resets the staging index entries for specified paths to match their state in `<rev>`.
- Does not modify HEAD or the working tree.
- Equivalent to un-staging changes while preserving working directory edits.

## `spine restore`

`spine restore` restores working tree files or staging index entries from a source revision.

### Flags and Targets

- `--staged`: Restores index entries from `--source` (defaults to HEAD).
- `--worktree`: Restores working-tree files from the staging index (or `--source` if specified).
- `--source <rev>`: Specifies the commit or tree object to restore from.
- `--all`: Restores all tracked files.

### Security Guarantees
- Paths are checked against directory traversal attacks (`..` components and absolute paths are rejected).
- Symlink escapes are validated and prevented.
