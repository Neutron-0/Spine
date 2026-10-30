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

## Milestone 2: Branching, Merging & Fast Working-Copy Operations
- [ ] Branch management (`spine branch create`, `spine branch list`, `spine branch delete`).
- [ ] Working tree checkout / switch (`spine switch`, `spine checkout`).
- [ ] Three-way merge engine with conflict markers and ancestor detection.
- [ ] Fast merge conflict resolution workflows.
- [ ] Operation log and undo journal (`spine undo`, `spine redo`).
- [ ] Stash / shelving mechanism.

---

## Milestone 3: Object Compaction, Packfiles & Network Protocol
- [ ] Multi-object packfile format with delta-compression (Rabin fingerprinting / content-defined chunking).
- [ ] Packfile indexing with fanout tables for sub-millisecond object resolution.
- [ ] Native Spine wire protocol over SSH and TLS.
- [ ] `spine remote`, `spine push`, `spine pull`, and `spine clone`.
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
