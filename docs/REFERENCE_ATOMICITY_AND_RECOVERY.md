# Reference Atomicity and Crash Recovery

## 1. Reference Publication via Atomic Lockfiles

Reference updates on disk (`refs/heads/<branch>`, `refs/tags/<tag>`) must never be observed in a partially-written or torn state.

Spine implements lockfile-based atomic publication:
1. When a reference update is committed, the target ref path is locked via `<ref_path>.lock`.
2. The full hexadecimal representation of the `ObjectId` followed by a newline is written to `<ref_path>.lock`.
3. The file descriptor is explicitly flushed and synced (`fsync`).
4. An atomic rename / file replacement operation (`spine_core::atomic_replace_file` or Win32 `MoveFileExW` with `MOVEFILE_REPLACE_EXISTING`) substitutes the `.lock` file over the target reference.
5. In case of error or failure before rename, the `.lock` file is removed, preserving the previous reference tip untouched.

## 2. Crash Recovery and Reconciliation

If a crash or power loss occurs during a distributed write:
1. **Crash during Phase 1 (Staging)**:
   - Staged packfiles reside in isolated `staging/<tx_id>` directories.
   - The coordinator marks the transaction aborted after expiry (`expires_at_nanos`).
   - Anti-entropy or background GC sweeps remove orphan staging directories.
2. **Crash between Phase 2 (Commit Decision) and Reference Publication**:
   - The coordinator holds the authoritative record of `CommitDecided`.
   - On coordinator reconciliation or client retry, the completed pack is already registered and verified on disk.
   - The writer detects that the merge commit or push tip is already in place and idempotently re-applies the reference pointer update.
3. **Crash after Reference Update but before DB Record**:
   - In PR merges, the route checks `state.vcs.resolve_ref(&repo.native_repo_id, &pr.base_branch)`.
   - If the current tip matches a merge commit whose parents are `(base_commit_id, head_commit_id)`, the merge is reconciled immediately without creating duplicate commits.
