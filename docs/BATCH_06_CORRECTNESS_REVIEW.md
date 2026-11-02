# BATCH 06 CORRECTNESS REVIEW & RECONCILIATION

## 1. Test Inventory Reconciliation

The Batch 06 engineering execution report noted a total test count of 86, but misallocated the breakdown between unit and integration tests. A line-by-line reproduction and discovery of all workspace test binaries yields the precise count:

### Unit Tests (Crates)
| Crate | Test Count | Location | Key Invariants Tested |
|---|---:|---|---|
| `spine-config` | 2 | `crates/spine-config/src/lib.rs` | Key-value settings, hash algorithm immutability |
| `spine-core` | 3 | `crates/spine-core/src/lib.rs` | Hex roundtrip, domain separation, BLAKE3 vs SHA-256 |
| `spine-diff` | 2 | `crates/spine-diff/src/lib.rs` | Tree comparisons, status calculation |
| `spine-format` | 8 | `crates/spine-format/src/lib.rs` | Annotated tags (`SPTG`), commits v1/v2, trees, loose headers |
| `spine-history` | 5 | `crates/spine-history/src/lib.rs` | Graph walk, merge base, criss-cross, bisect, blame |
| `spine-index` | 2 | `crates/spine-index/src/lib.rs` | Index serialization, checksum verification |
| `spine-journal` | 4 | `crates/spine-journal/src/lib.rs` | Two-phase prepare/commit, state machine, undo/redo |
| `spine-merge` | 3 | `crates/spine-merge/src/lib.rs` | 3-way clean text merge, conflict markers, tree merges |
| `spine-net` | 13 | `crates/spine-net/src/lib.rs` | Ed25519 identity, handshake, replay defense, TLS, object closure |
| `spine-object-store` | 7 | `crates/spine-object-store/src/lib.rs` | Loose store, repacking, fanout indexing, GC reachability |
| `spine-protocol` | 4 | `crates/spine-protocol/src/lib.rs` | Wire framing, message serialization, length limits |
| `spine-worktree` | 2 | `crates/spine-worktree/src/lib.rs` | Stash push/pop, multi-worktree lifecycle & branch locking |
| **Total Unit Tests** | **55** | | |

### Integration Tests (`apps/spine-cli/tests/`)
| Test Binary | Test Count | Scope |
|---|---:|---|
| `blame_bisect_test` | 1 | Line provenance attribution & binary search bisect |
| `branch_switch_test` | 1 | Safe branch switching & working tree preservation |
| `cli_e2e_test` | 1 | Full end-to-end repository lifecycle |
| `ed25519_auth_test` | 1 | Asymmetric Ed25519 authentication and roles |
| `fs_ops_test` | 1 | Filesystem helpers: mv, rm, clean |
| `gc_multi_worktree_test` | 1 | GC root protection across worktrees & stashes |
| `journal_recovery_test` | 2 | Crash recovery on fsck, multi-step undo/redo |
| `lease_push_test` | 1 | Atomic lease push & concurrency conflict rejection |
| `lifecycle_test` | 1 | Init, commit, branch, merge lifecycle |
| `merge_test` | 5 | Fast-forward, 3-way clean, conflict abort/continue |
| `network_remote_test` | 3 | Clone, fetch, push, pull, remote config, tags |
| `object_closure_test` | 1 | Push rejection on missing blob/tree closure |
| `protocol_hardening_test` | 3 | Frame bounds, state machine violations, path escapes |
| `rebase_test` | 1 | Linear history replay and --onto |
| `reset_restore_test` | 1 | Soft, mixed, hard reset & restore |
| `revert_cherry_pick_test` | 1 | Single commit replay & inverted revert |
| `stash_test` | 1 | Stack stash push/list/show/apply/pop/drop |
| `storage_v2_pack_gc_test` | 2 | Pack creation, compaction, fanout index, GC |
| `tag_annotated_test` | 1 | Annotated & lightweight tag creation & verification |
| `tls_transport_test` | 1 | TLS 1.3 handshake and secure data transfer |
| `worktree_test` | 1 | Multi-worktree add/list/remove, branch locking |
| **Total Integration Tests** | **31** | Across 21 test binaries |

**Reconciled Workspace Total: 55 unit tests + 31 integration tests = 86 tests passed, 0 failed, 0 ignored.**

---

## 2. High-Risk Functionality Audit

### A. Rebase
- **Supported Behavior**: Linear commit replay of commits between merge base and HEAD onto `<upstream>` (or `--onto <newbase>`).
- **Commit Metadata**: Original author name, email, and timestamp are preserved; committer name, email, and timestamp are updated to the active user. Original commit message is preserved.
- **Conflict Handling**: Pauses on merge conflict, emits conflict markers into working files, records `.spine/MERGE_STATE` and `.spine/rebase-apply/state.json`. `--continue` and `--abort` cleanly recover or restore the pre-rebase `orig_head`.
- **Limitation**: Spine rebase does **not** preserve merge topology (i.e. `--rebase-merges` is not supported). Rebasing a branch with merge commits flattens them into a linear commit sequence.

### B. Multi-Worktree Engine
- **Isolation**: Each linked worktree maintains its own independent `HEAD` and binary staging `index` inside `<primary_spine_dir>/worktrees/<id>/`.
- **Shared State**: Object storage (`objects/`, `pack/`), pack catalogs, references (`refs/heads/`, `refs/tags/`), and configuration (`spine.toml`) reside exclusively in the primary repository.
- **Branch Ownership**: A branch may only be checked out in exactly one worktree at a time. Attempts to checkout an already checked-out branch in another worktree fail with an explicit ownership error.
- **Pruning & Deletion**: Removing a linked worktree deletes only its administrative metadata directory and worktree files; shared packfiles and refs in the primary repository are completely unaffected.

### C. Stash
- **Commit Topology**: Modeled as multi-parent commits in the main object store. Parent 1 points to `HEAD`, Parent 2 to the staged index tree, and optional Parent 3 to untracked files (`-u`).
- **Stack Persistence**: Serialized into `.spine/stash/stashes.json`. Dropping or clearing removes the metadata record.

### D. Blame and Bisect
- **Blame**: Uses first-parent linear history walk to attribute line provenance. Merge commits are traversed along their first parent. Full merge-aware multi-parent graph attribution is deferred.
- **Bisect**: Partitions reachable DAG commits between bad and good commits using topological midpoints (`find_bisect_midpoint`).

### E. Garbage Collection
- **Roots**: Fully protects primary branches, tags, remote tracking branches, stashes, all linked worktree HEADs, all linked worktree staging indexes, and in-flight operation states (`rebase-apply`, `MERGE_STATE`, `CHERRY_PICK_STATE`, `REVERT_STATE`).
- **Annotated Tags**: `ObjectType::Tag = 0x04` objects are parsed during reachability traversal, recursively marking the `target_object` (commit, tree, or blob) as reachable.
