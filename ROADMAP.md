# SPINE DEVELOPMENT ROADMAP

## Milestone 1: Foundation, Research Corpus & Working VCS Vertical Slice (Batch 01 - Completed)
- [x] Reference Corpus acquisition and comparative analysis (`git`, `gitoxide`, `jj`, `heartwood`, `gitea`, `forgejo`, `git-lfs`).
- [x] Architectural documentation and ADRs (`ADR-0001-VCS-DATA-MODEL`).
- [x] Binary storage format specification (`FORMAT_SPEC.md`).
- [x] High-performance Rust VCS engine implementation:
  - `spine-core`: Object identifiers, domain separation, object types.
  - `spine-format`: Strict deterministic binary serialization.
  - `spine-object-store`: Content-addressed storage with BLAKE3 and Zstandard compression.
  - `spine-index`: Binary staging manifest and fast filesystem stat cache.
  - `spine-worktree`: Directory scanning, ignore rule evaluation, unstaged diff detection.
  - `spine-history`: Commit graph navigation, topological log ordering.
  - `spine-diff`: Fast Myers line-differencing.
  - `spine-config`: Hierarchical configuration parser.
  - `spine-integrity`: Comprehensive fsck validation.
  - `spine-cli`: Executable with commands: `init`, `config`, `add`, `status`, `commit`, `log`, `show`, `fsck`.
- [x] Spine Forge backend foundation (Axum + Tokio workspace).
- [x] Spine Forge frontend foundation (React + TypeScript + Vite).
- [x] End-to-end integration test suite, cross-platform validation, and performance benchmark baselines.

---

## Milestone 2: Branching, Merging & Fast Working-Copy Operations (Batch 02 - Completed)
- [x] Multi-parent commit architecture v2 with 100% backwards compatibility (`ADR-0002`).
- [x] Branch management (`spine branch`, `spine branch create`, `spine branch delete`, `spine branch rename`).
- [x] Preflight-checked safe working tree switching (`spine switch`, `spine switch -c`).
- [x] Three-way merge engine with LCA ancestor detection and 13 discrete reconciliation cases (`spine merge`).
- [x] Conflict marker generation (`<<<<<<< OURS`, `=======`, `>>>>>>> THEIRS`) and binary conflict detection.
- [x] Merge continuation and abort flows (`spine merge --continue`, `spine merge --abort`, `MERGE_STATE`).
- [x] Append-only operation journal (`spine-journal`, `ADR-0003`) with two-phase prepared/committed transactions.
- [x] Atomic undo and redo engine (`spine undo`, `spine redo`, `spine undo --list`).
- [x] Automatic crash recovery for interrupted operations.
- [x] Extended integrity verification (`spine fsck`) covering multi-parent commits, branch refs, merge state, and journal consistency.
- [x] Full integration and benchmark suites demonstrating 1.55x to 5.02x speedup over Git across all local workflows.

---

## Milestone 3: Storage Engine V2: Packfiles, Indexing, Compaction & GC (Batch 03 - Completed)
- [x] Immutable binary packfile format (`.spk`) with Zstandard compression and BLAKE3 framing (`ADR-0004`).
- [x] 256-bucket fanout binary search indexing (`.spidx`) for sub-millisecond object resolution.
- [x] Multi-pack generational catalog (`catalog.json`) with atomic publication protocol.
- [x] High-throughput loose-to-pack repacking engine (`spine repack`, `--dry-run`, `--verify`).
- [x] Conservative reachability-based garbage collection (`spine gc`, `--dry-run`, `--prune-before`).
- [x] Storage inspection and auditing commands (`spine count-objects`, `spine pack list`, `spine pack inspect`, `spine verify-storage`).
- [x] Durability hardening: 5-state operation journal (`Prepared`, `Applying`, `Committed`, `Aborted`, `RecoveryRequired`) and idempotent rollback.
- [x] Defect remediation: explicit conflict resolution staging tracking and multi-merge-base / criss-cross rejection (`AmbiguousMergeBases`).
- [x] Release-mode benchmarks demonstrating 1.59x to 17.14x speedup over Git (repack 6.01x, packed log 2.87x, gc 17.14x).

---

## Milestone 4: Native Wire Protocol & Remote Synchronization (Batch 04 - Completed)
- [x] Spine Wire Protocol (SPWP) v1 specification.
- [x] Standalone `spine-daemon` server with streaming protocol multiplexing.
- [x] Native remote commands: `clone`, `fetch`, `push`, `pull`.
- [x] Cryptographic object transfer verification and packfile streaming.

---

## Milestone 5: Native Network Security & Cryptographic Identity (Batch 05 - Completed)
- [x] Ed25519 peer identities and authentication handshake.
- [x] Pure Rust TLS 1.3 transport security.
- [x] Monotonic anti-replay protection and handshake challenge-response.
- [x] Lease-based reference update fencing and object-closure validation.

---

## Milestone 6: Advanced Local VCS & History Engine (Batch 06 - Completed)
- [x] Reset, restore, revert, cherry-pick, and rebase operations.
- [x] Fast stash engine with multi-entry stack.
- [x] Linked working trees (`spine worktree add/list/remove`).
- [x] Historical blame engine and binary bisect search.

---

## Milestone 7: Distributed Storage Fabric & Replication (Batch 07 - Completed)
- [x] Coordinator and multi-node storage clustering (`spine-coordinator`, `spine-storage-node`).
- [x] Multi-copy durability policies (SingleCopy, TwoCopies, ThreeCopies).
- [x] Fencing epochs, cluster node failover, and distributed repair.
- [x] Cluster administrative control CLI and monitoring.

---

## Milestone 8: Spine Forge Identity & Authentication (Batch 08 - Completed)
- [x] User registration, login, and Argon2id password hashing.
- [x] Session tokens and personal access tokens (PATs).
- [x] Outbox worker and repository provisioning pipeline.
- [x] Dual-backend database layer (`InMemoryDatabase` and `PgDatabase`).

---

## Milestone 9: Organizations, Collaboration, Issues & PRs (Batch 09 - Completed)
- [x] Organizations, teams, and multi-tenant access control.
- [x] Issues tracking, labels, milestones, and markdown comments.
- [x] Pull requests, reviews, inline comment threads, and branch protection rules.
- [x] Notifications system and repository activity streaming.

---

## Milestone 10: Distributed Write Transactions & Consistency (Batch 10 - Completed)
- [x] Two-phase commit write pipeline across distributed storage nodes.
- [x] Authoritative pull request merge execution.
- [x] Reference atomicity, conflict matrix reconciliation, and split-brain defenses.

---

## Milestone 11: Distributed Artifacts, LFS, Releases & Cargo Registry (Batch 11 - Completed)
- [x] Native Spine LFS engine replacing Git LFS with content-addressed chunk storage.
- [x] Releases and multi-asset distribution with quota management.
- [x] Crates.io-compatible Cargo package registry (`spine-api/index` + download endpoints).
- [x] Distributed artifact garbage collection and reachability preservation.

---

## Milestone 12: Forge Automation, Webhooks & CI/CD Runners (Batch 12 - Completed)
- [x] Transactional outbox webhook dispatch with HMAC-SHA256 and strict SSRF defenses.
- [x] Declarative workflow engine parsing DAG dependencies.
- [x] Ephemeral isolated build runners with environment wiping and process tree termination.
- [x] Fencing token lease reclamation and secret masking filters.

---

## Milestone 13: Search, Code Discovery, Discussions & Security Center (Batch 13 - Completed)
- [x] Permission-aware global & code search across snapshots, issues, PRs, and discussions.
- [x] Discussions platform with category hierarchies, Q&A accepted answers, and emoji reactions.
- [x] Developer & repository discovery: polyglot language breakdown, normalized topics, and extended developer profiles.
- [x] Security Center: Safe-fingerprinted secret scanning (zero raw token leakage), Cargo dependency audit, advisory matching, remediation workflows, and SPDX 2.3 / CycloneDX 1.5 SBOM export.
- [x] Full React 18 / TypeScript frontend integration in `App.tsx` and `types.ts`.
