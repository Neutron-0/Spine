# Spine Stash Architecture and Design

## Overview
`spine stash` records the current state of the working directory and staging index in a stack, reverting the working tree to a clean HEAD state without creating formal branch commits.

## Commit-Based Stash Storage

Spine models stashes as multi-parent commit structures stored in the primary object store:
- **Base Tree**: Matches the working directory modifications.
- **Parent 1 (`HEAD`)**: Current commit at the time of stashing.
- **Parent 2 (`Index Tree`)**: Staged state at the time of stashing.
- **Parent 3 (`Untracked Tree`, optional)**: Captured when `-u` / `--include-untracked` is specified.

## Metadata Persistence

Stashes are tracked in `<primary_spine_dir>/stash/stashes.json`:
```json
{
  "entries": [
    {
      "index": 0,
      "commit_id": "3a19b0c7...",
      "branch": "feature-auth",
      "message": "WIP on auth token refresh",
      "created_at_nanos": 1714651200000000000
    }
  ]
}
```

## CLI Subcommands

- `spine stash [push] [-m <msg>] [-u]`: Captures working tree and index into a new stash commit, prepends entry to `stash/stashes.json`, and resets working tree and index to HEAD.
- `spine stash list`: Displays all stashed states with index and branch provenance (`stash@{0}: WIP on main...`).
- `spine stash show [<index>]`: Displays diff between stash commit and its parent HEAD.
- `spine stash apply [--index] [<index>]`: Applies stashed tree onto current working directory; `--index` attempts to restore staged status.
- `spine stash pop [--index] [<index>]`: Applies stashed changes and drops the entry from the stash stack.
- `spine stash drop [<index>]`: Drops specified stash entry.
- `spine stash clear`: Removes all stash entries.

## Durability and GC Protection

All stash commits and their referenced trees and blobs are registered as roots during `spine gc` reachability computation, guaranteeing stash states are never pruned until explicitly dropped or cleared.
