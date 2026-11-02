# Spine Linked Multi-Worktree Architecture

## Overview
Spine supports multiple concurrent working trees linked to a single primary repository. This enables developers to work on multiple branches simultaneously without duplicating history or pack storage.

## Directory Layout

```
/path/to/primary-repo/
  .spine/
    HEAD                 -> ref: refs/heads/main
    index                -> primary staging index
    objects/             -> SHARED content-addressed object database
    pack/                -> SHARED packfiles & indexes
    refs/                -> SHARED branch, tag, and remote refs
    worktrees/
      feature-xyz/
        HEAD             -> ref: refs/heads/feature-xyz
        index            -> feature worktree staging index
        worktree.json    -> worktree metadata (path, lock state)

/path/to/linked-worktrees/feature-xyz/
  .spine                 -> file containing: "gitdir: /path/to/primary-repo/.spine/worktrees/feature-xyz"
  src/
  Cargo.toml
```

## CLI Operations

### Adding a Worktree
```bash
spine worktree add <path> [<branch>]
```
1. Validates that `<path>` does not exist or is empty.
2. Checks branch ownership: if `<branch>` is checked out in any other worktree (including primary), the command is aborted with an error.
3. If `<branch>` does not exist, creates `<branch>` in `<primary_spine_dir>/refs/heads/<branch>`.
4. Creates `<primary_spine_dir>/worktrees/<id>/` with dedicated `HEAD` and `index`.
5. Writes `.spine` pointer file in `<path>`.
6. Checks out commit root tree into `<path>`.

### Listing Worktrees
```bash
spine worktree list
```
Outputs canonical path, commit hash, and active branch for each registered worktree:
```
/path/to/primary-repo               3a19b0c [main]
/path/to/linked-worktrees/feature   f921d70 [feature-xyz]
```

### Removing & Pruning
- `spine worktree remove <id_or_path> [-f]`: Safely deletes linked directory and worktree administrative metadata.
- `spine worktree prune`: Cleans up administrative records for worktrees whose directories have been deleted from disk.

## Security & Path Isolation
Paths provided to `spine worktree add` are canonicalized and checked for symlink escapes. Branch ownership locks prevent destructive concurrent writes across worktrees.
