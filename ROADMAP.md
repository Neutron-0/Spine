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

---

## Milestone 17: Native Git LFS HTTP API, Smart HTTP Hardening, and Complete Large-File Remote Compatibility (Batch 17 - Completed)
- [x] Phase Zero audit and gap register (`BATCH_17_PHASE_ZERO_GAP_REGISTER.md`):
  - Multi-step `git fetch` and `git pull` lifecycle verification under official Git CLI oracle (`test_git_smart_http_fetch_pull_lifecycle`).
  - Hardened Git object mapping durability with atomic temporary file writes and filesystem renames (`save_git_map`).
  - Dynamic mapping reconciliation engine (`reconcile_git_map`) synthesizing valid Git OIDs for Spine-native commits.
  - Smart HTTP capability audit verifying strict compliance (protocol v2 explicitly unadvertised where not implemented).
  - Configurable canonical public origin (`public_origin`, `FORGE_PUBLIC_ORIGIN`, `APP_URL`) and plaintext Basic Auth rejection outside loopback.
- [x] Standard Git LFS HTTP Architecture (`routes_lfs.rs`):
  - Repository-scoped endpoints mounted at `/:owner/:repo_git/info/lfs/objects/batch`, `/:owner/:repo_git/info/lfs/objects/:oid`, and `/:owner/:repo_git/info/lfs/verify`.
  - Media type compliance: `application/vnd.git-lfs+json`.
  - Full Batch API supporting `upload` and `download` operations with `basic` transfer adapter.
  - Automatic deduplication: upload batch responses omit `actions` for already-present objects so Git LFS skips re-uploading them.
  - Per-object error reporting (HTTP 404 for missing download objects) without aborting entire batch operations.
- [x] Basic Transfer Implementation:
  - Streamed `PUT` upload endpoint streaming incoming binaries into `spine-artifact` content-addressed storage (4 MiB FastCDC chunks), verifying running SHA-256 and byte length.
  - Idempotent `POST` verification endpoint confirming persisted object existence and exact size.
  - Streamed `GET` download endpoint reconstructing raw binary bytes directly from `spine-artifact` chunk blocks with `Content-Type: application/octet-stream`.
  - Support for 1 GiB payload transfers via Axum `DefaultBodyLimit::max(1024 * 1024 * 1024)`.
- [x] Short-Lived Scoped Transfer Capabilities:
  - HMAC-SHA256 token generation and constant-time verification (`subtle::ConstantTimeEq`).
  - Scoped tokens strictly bound to repository ID, object SHA-256, operation, max size, and 1-hour expiration.
  - Prevention of cross-repository capability reuse, operation substitution, and OID tampering.
- [x] Smart HTTP & LFS Integration (Pre-Receive Hook Fencing):
  - Pre-receive inspection in `git_receive_pack` scanning newly introduced blobs for `GitLfsPointer`.
  - Automatic rejection of commits that introduce LFS pointers without the backing binary objects having been uploaded and verified.
- [x] Frontend UI Integration (`App.tsx`):
  - Clone dropdown updated with HTTPS Git Clone URL, Git LFS support details, and quickstart commands (`git lfs install`, `git lfs track "*.bin"`, `git add .gitattributes`).
  - Clean production build verified via Vite (`npm run build`).
- [x] Official Git and Git LFS End-to-End Test Suite (`git_lfs_http_test.rs`):
  - Batch API conformance testing across 10 discrete cases (`test_git_lfs_batch_api_conformance`).
  - Pre-receive missing LFS object push rejection test (`test_git_lfs_receive_pack_pre_receive_rejection`).
  - End-to-end official client integration test (`test_official_git_lfs_workflow`) using host `git 2.53.0.windows.1` and `git-lfs/3.7.1` tracking, committing, pushing, cloning, modifying, and pulling a 5 MiB binary asset with byte-for-byte verification.
- [x] Comprehensive documentation suite authored in `docs/`:
  - `BATCH_17_PHASE_ZERO_GAP_REGISTER.md`
  - `GIT_LFS_HTTP_ARCHITECTURE.md`
  - `GIT_LFS_BATCH_API_SUPPORT.md`
  - `GIT_LFS_BASIC_TRANSFER.md`
  - `GIT_LFS_AUTHORIZATION.md`
  - `GIT_LFS_STORAGE_MAPPING.md`
  - `GIT_SMART_HTTP_SECURITY.md`
  - `GIT_OBJECT_MAPPING_DURABILITY.md`
  - `GIT_REMOTE_HTTPS_CONFIGURATION.md`
  - `GIT_REMOTE_TEST_MATRIX.md`

---

## Milestone 18: Native Git-over-SSH, Key Management & File Locking (Batch 18 - Completed)
- [x] Pure Rust Git-over-SSH transport engine via `russh` (`spine_api::ssh`).
- [x] Public key authentication, deploy keys with read/write scoping, and constant-time signature verification.
- [x] Git LFS SSH authentication and locking protocol integration.
- [x] Verification with official Git CLI and Git LFS over SSH.

---

## Milestone 19: Production Packaging & Real PostgreSQL Verification (Batch 19 - Completed)
- [x] PostgreSQL database migration engine with transactional execution.
- [x] Deployment profiles (`core-forge-v1`, `development`).
- [x] Backup and restore subsystem (`SPNEBAK v1`).
- [x] Packaging and deployment rehearsal runbooks.

---

## Milestone 20: Release Candidate & First Codex Audit Closure (Batch 20 - Completed)
- [x] Session advisory locking in `run_migrations`.
- [x] Systemd unit files and Docker Compose configurations.
- [x] Production deployment rehearsals in clean environment.

---

## Milestone 21: Independent Codex Audit Remediation (Batches 21, 21A, 21B, 21C - Completed)
- [x] **F-01**: Streaming JSON Lines database dump and restore (Version 3.0), exact JSONB arbitrary numeric precision without floating point loss, database-backed cluster maintenance barrier wired to all mutation routes.
- [x] **F-02**: SPNEBAK v2 streaming encryption and failure-atomic key rotation with canonical same-path collision check, temporary staging, pre-publish header validation, and zero residual disk residue.
- [x] **F-03**: Persisted authenticated workflow provenance in PostgreSQL, enforced across trigger, scheduler, and runner assignment layers.
- [x] **F-04**: Narrowed PostgreSQL network subnet, established granular role separation (`spine_migrator` / `spine_app`), and verified runtime schema creation rejection.
- [x] **F-05**: Production Compose port segregation, HTTPS public origin enforcement, and credential userinfo rejection.
- [x] **F-06**: Password masking across all logs, strict secret file validation, and fail-fast empty target database checks.
- [x] **F-07**: Verified real PostgreSQL transaction-level advisory lock mutual exclusion, task cancellation, and recovery.

---

## Milestone 21D: System-Wide Writer Coordination, Recoverable Restore, Production Database Security and Release Candidate Verification (Batch 21D - Completed)
- [x] **F-01 (System-Wide Writer Coordination)**: Extended global maintenance barrier across all API mutating routes, background workers (`ProvisioningWorker`, `WorkflowScheduler`, `WebhookOutboxWorker`), and Git transports. Dedicated non-pooled session connections for PostgreSQL advisory locks (`0x5350494E_4542414B`), eliminating detached unlock in `Drop`. Enforced 16 MiB row limits on snapshot serialization.
- [x] **F-02 & F-06 (Durable Cross-Resource Restore)**: Two-phase staged restore state machine with durable journal (`.spine_restore_journal.json`). Isolated temporary staging (`.tmp_restore_<uuid>`), pre-cutover database verification, atomic storage publication only after target database restore succeeds, and automatic startup crash recovery.
- [x] **F-02 (Bounded Archive Processing)**: Removed unbounded memory allocation in legacy SPNEBAK v1 processing; enforced strict bounds on manifest size (16 MiB), total archive files (1,000,000), path lengths (4096 bytes), and archive sizes.
- [x] **F-03 (Authoritative Workflow Provenance at Executor Boundary)**: `IsolatedRunnerExecutor::execute_job` mandates non-forgeable `AuthoritativeTrustToken` derived from database lookup. Removed caller-asserted trust parameters.
- [x] **F-04 (Production PostgreSQL Role Separation & Privileges)**: Established distinct identities (`spine_admin`, `spine_migrator`, `spine_app`). Revoked `CREATE ON SCHEMA public` from runtime `spine_app`. Configured `docker/postgres/init-roles.sql`, `scripts/upgrade_postgres_roles.sql`, and `scripts/upgrade_postgres_roles.sh`.
- [x] **F-05 (Container Toolchain Alignment & Hardening)**: Aligned all Dockerfiles to MSRV `rust:1.88-bookworm`.
- [x] **F-06 (Elimination of Credential-Bearing CLI Arguments)**: Removed `--database-url`, `--target-database-url`, `--migration-database-url` from public CLI flags in favor of `--*-file` flags and environment variables.
- [x] **Disposable Database Safety Contract**: Implemented `HarnessDisposableDb` provisioning isolated ephemeral test databases with strict safety proofs and explicit rejection of pre-existing databases.
- [x] **F-07 (Migration Cancellation & Advisory Locking)**: Verified transaction-level advisory locks in parallel test runs.
- [x] **Release Evidence**: Built release binaries (`spine-api.exe`, `spine.exe`, `spine-coordinator.exe`, `spine-storage-node.exe`) under `target/release-candidate/` with SHA-256 checksums in `SHA256SUMS`.

---

## Milestone 21E: Cross-Service Backup Consistency, Crash-Recoverable Restore, Trust Enforcement and Production Provisioning (Batch 21E - Completed)
- [x] **F-01 (System-Wide Writer Coordination & Authoritative Data Inventory)**: Extended writer coordination across runner endpoints (`register_runner`, `heartbeat`, `acquire_job`, `ingest_logs`, `complete_job`) with active verification at commit boundaries. Documented authoritative cluster data inventory across all five persistent domains (`postgres_data`, `spine_storage`, `storage_1/2/3_data`, `coordinator_data`, `spine_ssh`) in `docs/DATA_INVENTORY.md`.
- [x] **F-02 & F-06 (Crash-Recoverable Restore State Machine & Fail-Closed Startup Safety)**: Eliminated split-brain vulnerability by enforcing generation identities across database (`_restore_generation`) and storage roots (`.spine_storage_generation`). Structured 6-phase restore state machine (`Initialized` -> `StagedStorage` -> `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`). Server startup fails closed immediately if an incomplete restore cutover or generation mismatch is detected.
- [x] **F-03 (Workflow Trust-Token Construction & Provenance Binding)**: Gated `AuthoritativeTrustToken::for_test` strictly behind `#[cfg(any(test, feature = "test-support"))]`, eliminating test factories from release builds. Bound tokens strictly to `run_id`, `job_id`, `repository_id`, and fork classification with fail-closed provenance validation.
- [x] **F-04 (Dynamic PostgreSQL Provisioning & Secret Support)**: Parameterized role initialization in `docker/postgres/01-init-roles.sh` with Docker secret files support (`POSTGRES_*_PASSWORD_FILE`), eliminating hardcoded passwords and database names. Maintained least privilege: `spine_admin`, `spine_migrator` (DDL owner), `spine_app` (runtime DML only, `REVOKE CREATE ON SCHEMA public`).
- [x] **F-05 (Production Network Boundary Hardening)**: Removed `spine-public` from `spine-frontend` in `docker-compose.yml`, ensuring frontend assets route exclusively through internal `spine-ingress-proxy` via TLS edge reverse proxy.
- [x] **F-07 & Test Safety (Disposable Database Harness Identity Fallback Removal)**: Removed silent fallback in `tests/common/mod.rs` where `app_pool` fell back to `migrator_pool` on connection failure. Real `spine_app` least-privilege connection verified in `postgres_least_privilege_test.rs`.
- [x] **Release Packaging & Manifest**: Rebuilt all release binaries (`spine-api.exe`, `spine.exe`, `spine-coordinator.exe`, `spine-storage-node.exe`) under `target/release-candidate/`, computed SHA-256 sums, updated findings register, and verified zero clippy/fmt defects.

---

## Milestone 21F: Multi-Volume Backup, Crash-Safe Restore, Production PostgreSQL Provisioning, and Authoritative Execution (Batch 21F - Completed)
- [x] **F-01 (Deployment-Wide Multi-Volume Backup Architecture)**: Expanded backup and restore scope to cover all 7 persistent deployment volumes: `postgres_data`, `spine_storage`, `storage_1_data`, `storage_2_data`, `storage_3_data`, `coordinator_data`, and `spine_ssh`. Integrated cluster snapshot RPCs (`CaptureVolumeSnapshot`, `StageVolumeRestore`, `CommitVolumeRestore`, `RollbackVolumeRestore`) into coordinator and storage nodes, packing participant volume snapshots into unified `SPNEBAK v2` archives. Updated canonical data inventory in `docs/DATA_INVENTORY.md`.
- [x] **F-02 (Monotonic Fencing Barrier & Mutation Guards)**: Implemented 64-bit monotonic fencing generation barrier backed by PostgreSQL advisory lock `0x5350494E_4542414B` and `_cluster_maintenance_barrier`. Guarded storage nodes (`FileSystemStorageBackend`) and coordinator store (`FileBackedCoordinatorStore`), rejecting mutations and pack commits exhibiting stale fencing generations (`StaleFencingGeneration` / `InMaintenanceBarrier`).
- [x] **F-03 (Whole-Generation Directory Replacement)**: Eliminated absent-file survival by switching restore from file-by-file overlay to atomic whole-generation directory replacement. Live root is atomically archived to `.old_generation_<id>` prior to publishing staged trees, guaranteeing absent files from previous generations are removed, with atomic rollback on failure.
- [x] **F-04 (Transactional Generation Token & Fail-Closed Startup Recovery)**: `_restore_generation` token recorded inside active database restore transaction prior to commit. Upgraded `handle_startup_recovery` to fail closed on catalog query failures, unmigrated/corrupted state, or generation mismatches between database and storage.
- [x] **F-05 (Authoritative Trust Token in Execution Path)**: Enforced `AuthoritativeTrustToken::from_database` in `acquire_job` and `complete_job` runner services. Removed empty repository ID bypass in `IsolatedRunnerExecutor`.
- [x] **F-06 (Production PostgreSQL Roles & Secret Quoting)**: Distinct credentials enforced for `spine_admin`, `spine_migrator`, and `spine_app` without fallbacks in `01-init-roles.sh`. Quoted identifiers and literals with `format('%I', '%L')` and implemented non-destructive `has_schema_privilege` probe in `upgrade_postgres_roles.sh`.
- [x] **F-07 (Production Compose Container Hardening)**: Added `security_opt: [no-new-privileges:true]`, distinct secrets, and healthcheck via `spine_app` in `docker-compose.prod.yml`.
- [x] **F-08 (Disposable Test Database Safety Contract)**: Strict `SPINE_ALLOW_DESTRUCTIVE_TESTS=1` gating and eliminated fallback to administrative credentials in test pools.
- [x] **F-09 (Release Candidate Packaging & Checksums)**: Recompiled all 4 release binaries (`spine-api.exe`, `spine-coordinator.exe`, `spine-storage-node.exe`, `spine.exe`) with release profile, recorded SHA-256 sums in `RELEASE_CANDIDATE_MANIFEST.md` and `target/release-candidate/SHA256SUMS`. Full test suites passing with zero clippy/fmt warnings.

