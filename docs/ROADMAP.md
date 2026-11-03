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

### Batch 08: Spine Forge Identity, Authentication, Repository Provisioning, and Native Repository Hosting
- Rust backend `spine-api` using Axum 0.7, Tokio, and SQLx.
- Dual database abstraction (`Database` trait): `PgDatabase` (production PostgreSQL) and `InMemoryDatabase` (zero-external-dependency test runner).
- Cryptographic identity: Argon2id password hashing with random salt, constant-time verification, strict complexity validation.
- Session & Token engine: CSPRNG 32-byte tokens prefixed with `sp_sess_` and `sp_pat_`, BLAKE3 hash storage, sliding expiry, atomic revocation.
- Granular authorization (`PermissionService`): Reader, Collaborator, Owner, and Admin roles.
- Private repository metadata isolation: unauthorized requests to private repositories strictly return `404 Not Found` to prevent discovery of confidential project names.
- Asynchronous repository provisioning outbox (`repository_provisioning_jobs`) with idempotency keys and background worker (`ProvisioningWorker`).
- Coordinator RPC integration: `AllocateRepository` and `DeleteRepository` protocol requests with rack-aware placement plan creation.
- Native VCS inspection: `NativeVcsService` directly inspects on-disk `.spine` repositories (branches, tags, commits, tree navigation, blob content) via `CompositeObjectStore` with zero Git runtime execution.
- React 18 / TypeScript / Vite frontend: responsive developer UI for authentication, repository creation, branch switching, directory browsing, and commit history.
- Native CLI integration: `spine auth login/status/logout` and `spine repo create` via a custom zero-dependency async HTTP client.
- Production Docker Compose topology (`docker-compose.yml`), `.env.example`, and local runner scripts (`scripts/dev_forge.ps1`, `scripts/dev_forge.sh`).

---

## Next Milestones

### Batch 09: Pull Requests, Code Review, Inline Comments, and Mergeability Engine
- Pull request data models, source/target branch references, and automated 3-way mergeability checks.
- Unified and side-by-side diff viewers for commit and branch comparisons.
- Threaded code review discussions with line-level code comments and resolution tracking.
- Branch protection rules (required reviews, passing CI checks, linear history).

### Batch 10: Real-time Collaboration, Webhooks, CI/CD Pipeline Gateway, and Large Object Storage (LFS)
- Real-time updates via WebSockets for collaborative review and notification feeds.
- Webhook dispatch system with HMAC-SHA256 payload signing and retry queues.
- Spine LFS: Content-addressed external binary asset store with chunked streaming and content-hash verification.
- Partial and sparse worktree support for monorepo development.
