# ADR-0003: Append-Only Operation Journal, Undo/Redo Engine, and Crash Recovery

## Status
Accepted

## Date
2026-10-31

## Context
Traditional version-control systems (such as Git) maintain reference logs (`reflog`) which record individual reference movements (HEAD and local branch tips). However, reflogs suffer from critical limitations:
1. **Lack of Transactional Atomicity:** If an operation like `switch` or `merge` fails or is killed midway, the working tree, index, and references may be left in an inconsistent, partially modified state.
2. **No First-Class Operation Records:** Reflogs only record pointers; they do not capture the complete before-and-after snapshot of the repository (HEAD ref, branch tip, index checksum, worktree state, and operation parameters).
3. **No Coordinated Undo/Redo:** Undoing a complex operation (such as an accidental merge or switch) requires manual reconstruction using reset commands, which can easily lose unstaged work or commit pointers.
4. **Crash Recovery Gap:** There is no built-in protocol to detect and cleanly roll back interrupted operations.

Modern next-generation systems like Jujutsu demonstrate the power of an operation repository (`jj op log` and `jj undo`), where every mutating operation is treated as an explicit, verifiable transition.

Spine requires a dedicated, append-only operation journal that guarantees transactional integrity, provides crash recovery for interrupted operations, and supports native `spine undo` and `spine redo` commands.

## Decision

1. **Storage Layout in `.spine/journal/`:**
   - `.spine/journal/records/<seq>-<txid>.json`: Individual atomic transaction records containing full transaction metadata, status, before-state, and after-state.
   - `.spine/journal/HEAD`: A pointer file containing the transaction ID of the current active journal head.
   - All filesystem operations within the journal use atomic write-and-rename semantics (`tempfile` + `std::fs::rename`) to prevent partial writes.

2. **Transaction Lifecycle (Two-Phase State):**
   - **Phase 1: Prepared (`TransactionStatus::Prepared`)**:
     - Created before mutating repository state.
     - Captures:
       - Unique monotonic sequence number + UUID.
       - Timestamp.
       - Operation kind (`Commit`, `BranchCreate`, `BranchDelete`, `BranchRename`, `Switch`, `Merge`, `MergeAbort`, `Undo`, `Redo`).
       - `before_state`: HEAD reference, branch name, branch tip `ObjectId`, index checksum / snapshot.
       - Human-readable summary (e.g. `commit: Add feature X`, `switch: main -> feature-a`).
     - Written to disk and flushed before performing repository mutations.
   - **Phase 2: Committed (`TransactionStatus::Committed`)**:
     - Written after all repository changes (objects, refs, index, worktree) succeed.
     - Captures:
       - `after_state`: New HEAD reference, branch tip `ObjectId`, index checksum.
     - Sets journal `HEAD` to this transaction.
   - **Aborted (`TransactionStatus::Aborted`)**:
     - If an error occurs during execution, or if crash recovery detects an interrupted `Prepared` transaction, the status transitions to `Aborted`.

3. **Crash Recovery Protocol:**
   - Every mutating command and `spine fsck` initializes the journal engine.
   - The engine checks for any transaction in `Prepared` status that was never finalized.
   - When found:
     - The recovery engine restores the repository's `before_state` (HEAD, branch ref, index).
     - Marks the transaction record as `Aborted` with reason `"Interrupted by crash or kill"`.
     - Ensures the repository is always restored to a consistent pre-transaction state.

4. **Undo and Redo Semantics:**
   - `spine undo`:
     - Inspects the active journal HEAD.
     - Validates that the working tree and index are clean (or safe to restore).
     - Restores the `before_state` of the target transaction.
     - Records a new transaction of kind `Undo` that links to the undone transaction ID.
     - Moves journal active pointer backwards.
   - `spine redo`:
     - If the latest operation was an `Undo`, reapplies the target transaction's `after_state`.
     - Records a new transaction of kind `Redo`.
     - Moves journal active pointer forward.

## Consequences

- **Safety:** Repository state transitions are strictly atomic and crash-resilient across power failures or process kills.
- **Auditing:** Complete, tamper-evident history of all local operations is preserved in `.spine/journal/`.
- **Integrity Verification:** `spine fsck` verifies the integrity of the journal chain, ensuring sequence numbers and state hashes match.
- **Performance Overhead:** Journal record writes are small JSON payloads (< 2 KB) on SSD/NVMe drives (< 0.5 ms), adding negligible overhead compared to object storage.
