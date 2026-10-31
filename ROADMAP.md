# SPINE DEVELOPMENT ROADMAP

## Milestone 1: Foundation, Research Corpus & Working VCS Vertical Slice (Current Batch)
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

## Milestone 4: Network Synchronization & Remote Protocol (Batch 04)
- [ ] Native Spine wire protocol over SSH and TLS.
- [ ] Remote reference advertisement and negotiation.
- [ ] Packfile streaming and bundle generation for `spine push`, `spine pull`, `spine clone`.
- [ ] Cryptographic commit and tag signing (Ed25519 / SSH keys).

---

## Milestone 4: Spine Forge MVP (Developer Platform)
- [ ] Multi-tenant user authentication (Argon2id, Session cookies, PATs, OAuth2).
- [ ] Organization and team authorization models.
- [ ] Public and private repository hosting backed by `spine-storage-node`.
- [ ] Web repository browser: Tree view, raw file display, syntax highlighting, commit history.
- [ ] Issues tracker: Markdown descriptions, labels, milestones, assignments.
- [ ] Pull Requests & Code Reviews: Inline commenting, unified diff viewer, mergeability checker.

---

## Milestone 5: Distributed Storage, Large Files & CI/CD
- [ ] Distributed storage clustering with Raft consensus for placement and replication.
- [ ] Native large-file chunking and deduplication (replacing external Git LFS).
- [ ] Ephemeral sandboxed CI/CD runner execution engine.
- [ ] Static site and documentation preview publishing.
