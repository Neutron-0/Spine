# SPINE LFS CLI REFERENCE

## Overview
The `spine lfs` command suite manages tracked large files, pointers, local chunk stores, and remote synchronization.

---

## Commands

### 1. `spine lfs track <pattern>`
Registers a file pattern to be tracked via LFS. Appends the rule to `.spinelfs`.

```bash
$ spine lfs track "*.psd"
Tracking "*.psd" with Spine LFS. Added to .spinelfs

$ spine lfs track "weights/*.onnx"
Tracking "weights/*.onnx" with Spine LFS. Added to .spinelfs
```

---

### 2. `spine lfs untrack <pattern>`
Removes a pattern from `.spinelfs`.

```bash
$ spine lfs untrack "*.psd"
Untracked "*.psd" from Spine LFS. Removed from .spinelfs
```

---

### 3. `spine lfs status`
Displays all configured LFS patterns and staged LFS files in the current worktree.

```bash
$ spine lfs status
Spine LFS Patterns:
  - *.psd
  - weights/*.onnx

Tracked LFS files in repository:
  design/hero.psd (42.5 MB) [manifest: 8f9a2b...]
```

---

### 4. `spine lfs ls-files`
Lists all tracked files in the current commit and index that contain LFS pointers.

```bash
$ spine lfs ls-files
8f9a2b7c4d... * design/hero.psd (44564480 B)
3a1b0c9e8f... * weights/resnet50.onnx (102400000 B)
```

---

### 5. `spine lfs verify`
Scans all LFS pointers in the current index and commit trees to verify that all referenced chunks exist in the local artifact store and that their BLAKE3 checksums are valid.

```bash
$ spine lfs verify
Verifying LFS objects...
  [OK] design/hero.psd (11 chunks verified)
  [OK] weights/resnet50.onnx (25 chunks verified)
All LFS objects verified successfully.
```

---

### 6. `spine lfs prune [--dry-run]`
Identifies and purges unreferenced chunks from `.spine/artifacts/chunks/`. Uses `ArtifactGcCollector` to trace reachability from all repository commits, tags, stashes, and worktrees.

```bash
$ spine lfs prune --dry-run
Reachable chunks: 36 (144.0 MB)
Candidate orphan chunks: 4 (16.0 MB)
Dry run: no chunks deleted.

$ spine lfs prune
Purged 4 unreferenced chunks (16.0 MB freed).
```
