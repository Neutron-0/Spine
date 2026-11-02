# Spine Master Roadmap & Architectural Progression

## Completed Milestones

### Batch 01: Foundation, Object Store & First Working Engine
- Native `.spine/` repository format and BLAKE3 / SHA-256 content-addressing.
- Zstandard object compression for blobs, trees, and commits.
- Binary staging index and basic CLI (`init`, `config`, `add`, `status`, `commit`, `log`, `show`, `fsck`).

### Batch 02: Branching, Switching, Merging & Journaling
- Full branch management and safe working-tree switching (`switch`, `branch`, `diff`).
- 3-way tree merge with conflict detection, marker generation, and resolution.
- Multi-parent commit support.
- Two-phase transaction operation journal (`undo`, `redo`).

### Batch 03: Storage Engine V2
- Native `.spk` packfiles and `.spidx` fanout binary indexes.
- Multi-pack catalog (`packs.catalog`) and composite object store.
- Object repack and conservative garbage collection (`spine repack`, `spine gc`).

### Batch 04: Native Wire Protocol & Networking
- Spine Wire Protocol (SPWP) v1 with length-delimited framing and BLAKE3 checksums.
- `spine-daemon` server runtime.
- Remote repository operations (`clone`, `fetch`, `push`, `pull`, `remote`).

### Batch 05: Cryptographic Identity & Transport Security
- Ed25519 asymmetric peer identity and cryptographic challenges.
- TLS 1.3 transport security over raw TCP.
- Replay attack defenses and lease-based atomic force-push (`--force-with-lease`).
- Object-closure graph verification before push acceptance.

### Batch 06: Advanced Local VCS, History Manipulation & Multi-Worktree Engine
- History manipulation: `reset` (soft, mixed, hard), `restore`, `revert`, `cherry-pick`.
- Linear commit replay engine: `rebase` (`--onto`, `--continue`, `--skip`, `--abort`).
- Stack-based stash management: `stash` (`push`, `list`, `show`, `apply`, `pop`, `drop`, `clear`).
- Linked multi-worktree engine: `worktree` (`add`, `list`, `remove`, `prune`) with shared pack storage and branch ownership locks.
- Annotated cryptographic tags: `tag` (`ObjectType::Tag = 0x04`).
- Inspection & Localization: `blame` (line provenance) and `bisect` (binary search defect isolation).
- Workspace filesystem helpers: `mv`, `rm`, `clean`.
- GC protection across worktrees, stashes, tags, and in-flight states.

---

## Next Milestones

### Batch 07: Distributed Storage Fabric, Repository Replication, Node Management, Failover and Data Recovery
- Crate `spine-cluster`: Node lifecycle state machine, Ed25519 challenge-response enrollment, autonomous heartbeats, and failure domain tracking.
- Durability policies (`SingleCopy`, `TwoCopies`, `ThreeCopies`) enforced via two-phase commit replication transactions (`Initiated` -> `Preparing` -> `Prepared` -> `CommitDecided` -> `Committed`).
- Primary ownership epochs and fencing tokens preventing split-brain writes.
- Automatic failover coordinator verifying health, committed generation, and cryptographic integrity before replica promotion.
- Anti-Entropy reconciliation diffing immutable pack catalogs and streaming repair packs.
- Distributed GC tombstones with mandatory retention grace periods and committed generation checks.
- Production PostgreSQL metadata schema (`migrations/20261102000001_cluster_metadata.sql`) and `CoordinatorStore` abstraction with `FileBackedCoordinatorStore`.
- Storage Node daemon (`spine-storage-node`) and Distributed Coordinator daemon (`spine-coordinator`).
- Cluster administration CLI: `cluster status`, `node list/inspect/drain/quarantine/retire`, `storage placements/inspect/rebalance`, `replication status/repair`.
- Multi-process local dev cluster automation (`scripts/dev_cluster.ps1`, `scripts/dev_cluster.sh`).

---

## Next Milestones

### Batch 08: Spine Large Object Storage (LFS) & Partial Worktrees
- Spine LFS: Content-addressed external binary asset store with chunked streaming and content-hash verification.
- Sparse checkouts and path-filtered indexes for massive monorepos.

### Batch 09: Spine Forge Architecture & Rust Backend
- Forge daemon with multi-tenant repository hosting.
- Role-based access control, organizational permissions, and SSH/SPWP gateway.
- Pull request engine with automated mergeability testing and review commenting.

### Batch 10: Spine Forge Web Frontend
- High-performance TypeScript / WebAssembly repository browser, code viewer, and graph visualizer.
- Real-time review and collaborative workflow interface.
