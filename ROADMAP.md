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

---

## Milestone 14: Full-System Integration, Security Hardening, Reliability & Performance (Batch 14 - Completed)
- [x] System Integration Register (`BATCH_14_SYSTEM_INTEGRATION_REGISTER.md`) across all 19 subsystems.
- [x] Batch 13 test failure reconciliation: fixed `forge_e2e_test.rs` type mismatch, bounded job concurrency (`-j 2`) to eliminate Windows OS error 1455, verified full test suites without assertion suppression.
- [x] Full workspace test inventory: 124 passing Rust tests across VCS and Forge backend, 0 failed, 0 skipped; clean frontend TypeScript production build.
- [x] Database reliability audit: truthful reporting on PostgreSQL environment availability (BLOCKED on host server/Docker); transactional boundaries, CAS locking, and migration integrity validated.
- [x] Runner execution security: fail-closed enforcement for unsandboxed untrusted workflows, scoped capability credentials, withheld secrets on forked PRs, process-tree termination.
- [x] Authoritative VCS write pipeline: verified compare-and-swap ref atomicity, object closure, and anti-entropy repair.
- [x] Webhook SSRF mitigation: connection-boundary validation with pinned IP sockets, zero redirects, prohibited port enforcement, and HMAC-SHA256 constant-time verification.
- [x] Secret scanning hardening: keyed HMAC-SHA256 (`hmac-sha256:v1`) fingerprinting with key rotation support, masked previews, zero plaintext leakage.
- [x] Cargo vulnerability intelligence: advisory feed provenance and freshness tracking.
- [x] Search consistency & authorization: query-layer ACL filtering and bounded line snippet buffers.
- [x] Observability and operations: `/health/live`, `/health/ready`, and `/health/diagnostics` endpoints.
- [x] Reproducible performance baselines: 1.24x to 12.00x speedup over Git across native VCS operations.
- [x] Failure injection test suite (`failure_injection_test.rs`) and comprehensive disaster recovery runbook.

---

## Milestone 15: Native Interoperability, Migration, Import/Export & Ecosystem Compatibility (Batch 15 - Completed)
- [x] Pure Rust native Git format engine in `git-alternative/crates/spine-migration/` with zero external Git / libgit2 dependencies in production.
- [x] Loose Git object parser and serializer for blobs, trees, commits, and tags with zlib compression and SHA-1 hash checks.
- [x] Canonical Git tree sorting with synthetic trailing `/` directory comparison, verified with official `git fsck --full`.
- [x] Git packfile v2 parser, LEB128 delta instruction decoder (COPY and INSERT), and recursive OFS_DELTA and REF_DELTA resolver.
- [x] Git packfile builder and v2 index generator with 256-bucket fanout table, lexicographical SHA-1s, CRC32 table, offset table, and SHA-1 checksum trailers.
- [x] Git reference reader and writer with loose refs, HEAD parsing, and `packed-refs` format support.
- [x] Fast-import and fast-export streaming protocol parser and generator with mark tracking.
- [x] Git LFS pointer parser, detection, and hydration into Spine's content-addressed artifact store.
- [x] Hardened archive extraction engine (`archive_security.rs`) defending against Zip Slip, directory traversal, Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`), and decompression bombs.
- [x] Platform metadata importer for GitHub and GitLab archive bundles (issues, PRs, comments, labels, milestones, releases).
- [x] Safe user attribution and ghost tombstone user mapping (`ghost-<username>`) preventing privilege escalation.
- [x] Fail-closed CI/CD workflow security: imported workflows default to `is_active: false`.
- [x] Resumable migration orchestrator (`MigrationEngine`) with inspect, plan, dry-run, run, manifest checkpoints, resume, and verify.
- [x] CLI commands in `spine-cli`: `spine import git`, `spine export git`, `spine migrate inspect/plan/run/resume/verify/report`.
- [x] Forge REST API endpoints under `/api/v1/migration/*` (inspect, plan, run, export).
- [x] React 18 / TypeScript frontend UI with dedicated Migration tab, source inspector, planning report, progress tracker, and Git export.
- [x] Roundtrip export verified against official host Git CLI oracle (`git fsck --full`, `git log`, `git cat-file`).
- [x] 13 comprehensive architectural, operational, and format documentation guides in `docs/`.

---

## Milestone 16: Native Git Smart HTTP, Remote Clone/Fetch/Push, and Forge Transport Integration (Batch 16 - Completed)
- [x] Phase Zero gap closure from Batch 15 (`BATCH_16_PHASE_ZERO_GAP_REGISTER.md`):
  - Strictly lexicographical `packed-refs` sorting ensuring clean validation by `git fsck --full`.
  - Resilient platform metadata deserialization supporting string and object user structures (`PlatformUser`).
  - Genuine LFS chunk hydration connecting Git LFS pointers to `spine-artifact` content-addressed storage.
  - Hardened archive extraction security defending against symlink/hardlink escapes and case-insensitive duplicates.
  - Deterministic migration checkpoint resumption and reconciliation verification.
- [x] Pure Rust Git Smart HTTP protocol transport (`pkt_line.rs`, `git_transport.rs`) with zero external Git or libgit2 runtime dependencies:
  - Canonical packet-line framing, flush (`0000`), delimiter (`0001`), and end-of-response (`0002`) packets.
  - Protocol v1 and v2 capability advertisement (`multi_ack_detailed`, `side-band-64k`, `thin-pack`, `ofs-delta`, `agent`, `report-status`).
  - Read transport: `GET /info/refs?service=git-upload-pack` and `POST /git-upload-pack` with wants/haves reachability negotiation and sideband channel 1 streaming.
  - Write transport: `GET /info/refs?service=git-receive-pack` and `POST /git-receive-pack` with atomic ref update commands and `report-status` responses.
- [x] Ingest & Thin Pack Delta Resolution:
  - Built-in external base object resolver for thin packs resolving `OFS_DELTA` and `REF_DELTA` bases against existing repository commits, trees, and blobs.
  - Git-to-Spine and Spine-to-Git bidirectional object conversion with cached raw Git objects and persisted `git_map.json` mapping.
  - Bit-for-bit Git commit SHA-1 preservation guaranteeing zero history divergence during subsequent `git pull` operations across independent Git clients.
- [x] Forge API Git Smart HTTP routes (`routes_git_http.rs`) mounted at `/:owner/:repo_git/info/refs`, `/:owner/:repo_git/git-upload-pack`, and `/:owner/:repo_git/git-receive-pack` supporting both `.git` and bare repository path suffixes.
- [x] HTTP Basic Authentication & Access Control:
  - Constant-time verification supporting both Personal Access Tokens (`sp_pat_...`) and user passwords (Argon2id).
  - Public repository anonymous read discovery/clone and authenticated write gating.
  - Private repository authentication enforcement across all read and write endpoints.
- [x] Pre-Receive Security & Consistency Pipeline:
  - Immediate blob scanning against known secret patterns prior to ref updates, rejecting offending pushes with descriptive client-visible error lines.
  - Pre-receive branch protection rules enforcement (`require_pull_request`, fast-forward check, deletion gating).
  - Authoritative CAS ref publication ensuring atomic, conflict-free repository state transitions.
- [x] Frontend UI Integration (`App.tsx`):
  - Interactive `Clone` dropdown in the repository Code tab providing canonical Git Smart HTTP URLs, copy-to-clipboard functionality, and PAT usage guidelines.
- [x] End-to-end integration test suite (`git_smart_http_test.rs`) using host Git CLI (`git 2.53.0.windows.1`) as external oracle:
  - Verified initial push, clone roundtrip, and incremental commit push/pull with bit-for-bit file tree verification.
  - Verified private repository authentication gating and PAT authorization.
  - Verified branch protection rule enforcement and push rejection.
  - Verified pre-receive secret scanning rejection of credential leakage.
  - Verified clone and push via bare repository URLs (omitting `.git`).
- [x] Architecture, protocol, authentication, push security, pack transfer, and operational runbook documentation in `docs/`.
