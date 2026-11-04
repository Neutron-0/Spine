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

### Batch 09: Spine Forge Organizations, Collaboration, Issues, Pull Requests, Code Reviews, and Notifications
- First-class organizational multi-tenancy: `organizations`, `organization_memberships`, and auditable `organization_invitations` with role hierarchies (`Owner`, `Admin`, `Member`) and last-owner protection.
- Teams and granular RBAC: `teams`, `team_memberships`, and `team_repository_permissions` with 6-level permission evaluation (`ADMIN` > `MAINTAIN` > `WRITE` > `READ`) and strict `404 Not Found` metadata leakage prevention.
- Issue tracking & metadata: `issues`, `issue_assignees`, `issue_labels`, `issue_comments`, `repository_labels`, and `milestones`.
- Automatic issue closing: PR titles and descriptions scanned for closing keywords (`Fixes #123`, `Closes #456`) executing atomic status transition upon merge.
- Pull request engine: `pull_requests` and `pull_request_reviewers` tracking branch divergence, automated mergeability analysis, and state transitions (`OPEN`, `CLOSED`, `MERGED`).
- Native VCS diff & mergeability: zero Git/libgit2 execution; tree flattening, Myers unified diff formatting (`spine-diff`), fast-forward detection, and 3-way tree conflict analysis (`spine-merge`).
- Code review subsystem: `pull_request_reviews` (`APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`), line-anchored `pull_request_review_comments`, self-approval prevention, and thread resolution tracking.
- Branch protection gatekeeper: `branch_protection_rules` enforcing PR requirements, approval thresholds, self-approval prevention, and unresolved discussion blocking before merge.
- Atomic 3-way merge execution: creation of native two-parent `Commit` objects and atomic `.spine/refs/heads/<base>` ref updates via `.lock` safety.
- Notification & activity feed: `user_notifications` (`REVIEW_REQUESTED`, `ASSIGNED`, `MENTIONED`, `MERGED`) with read state tracking and immutable append-only `repository_activity_events` audit trail.
- Full React 18 + TypeScript frontend collaboration suite: Code browsing, Issues management, Pull Requests with unified diffs, Organizations & Teams administration, Branch Protection configuration, and real-time Notification drawer.

### Batch 10: Native Hosting Completion, Authenticated Remote Operations, Distributed Write Transactions, and Repository Consistency
- Single Authoritative Write Pipeline (`AuthoritativeRepositoryWriter`): Unified write boundary orchestrating all repository modifications across CLI `push`, wire SPWP transport, and Forge PR merges.
- Cryptographic Capability Tickets (`CapabilityTicket`): Short-lived BLAKE3 signed tokens issued via `POST /api/v1/repos/:owner/:repo/capability` binding caller identity, native repository ID, and operation role (`Reader`, `Contributor`, `Admin`).
- SPWP Server Capability Auth Integration: Daemon authenticates incoming `ticket:<hex>.<sig>` sessions and enforces strict repository namespace containment (mismatched repository path rejected with `403 Forbidden`).
- OS-Native Secure Credential Storage (`OsCredentialManager`): Win32 `advapi32.dll` (`CredWriteW`, `CredReadW`, `CredDeleteW`) integration via `WindowsCredentialStorage`, with zero raw token terminal leakage (token masking redaction) and restricted keyring file fallback (`SecureFallbackStorage`).
- Unified 2PC Distributed Write Transactions: Phase 1 staging on Primary and Replicas (`PreparePush`), quorum verification against `DurabilityPolicy` (`SingleCopy`, `TwoCopies`, `ThreeCopies`), and Phase 2 commit decision (`CommitPush`) with atomic `.lock` reference publication.
- Durability Status Verification: Endpoint `GET /api/v1/repos/:owner/:repo/status/durability` returning real-time copy verification reports (`DurabilityVerificationReport`).
- PR Merge Distributed Integration: PR merges directly construct genuine `.spk` and `.spidx` packfiles from 3-way tree and commit objects, submitting through `AuthoritativeRepositoryWriter` with cross-service crash reconciliation.
- Comprehensive Test Suites: 109 VCS/CLI tests and 25 Forge API tests passing with zero failures.

### Batch 11: Distributed Large Object Storage, Spine LFS, Releases, Artifact Management, and Package Registries
- Unified Content-Addressed Storage Crate (`spine-artifact`): Deterministic 4 MiB bounded streaming chunker (`StreamChunker`), whole-object and per-chunk BLAKE3 digests, cryptographic manifests (`ArtifactManifest`), and memory-bounded reconstructor (`ObjectReconstructor`).
- Spine LFS Engine: Text pointer format (`spine-lfs-v1 <manifest_oid> <size_bytes> <object_hash>`), `.spinelfs` tracking rules file, worktree interception in `spine add` staging compact pointers instead of multi-gigabyte blobs, and full CLI commands (`spine lfs track`, `untrack`, `status`, `ls-files`, `verify`, `prune`).
- Distributed Cluster Protocol Extension: Generalized `spine-cluster` and `spine-storage-node` with atomic chunk (`StoreChunk`, `GetChunk`) and manifest (`StoreManifest`, `GetManifest`) operations, orchestrated across storage nodes by `DistributedArtifactStore` with quorum-based durability policies (`SingleCopy`, `TwoCopies`, `ThreeCopies`).
- Resumable Transfer Protocol: High-throughput chunk streaming with session state machine (`Created` -> `Uploading` -> `Finalizing` -> `Completed`), TTL timeouts, and pre-allocation quota validation.
- Capacity Accounting & Mark-and-Sweep GC: Dual logical vs physical deduplicated disk accounting (`artifact_quotas`), tenant limits (default 10 GiB), reachability tracing from active commit LFS pointers, releases, packages, and namespaces, with a 24-hour retention grace period.
- Forge Releases & Assets: Draft and published release lifecycles, immutable tag and commit bindings, binary release asset distribution with atomic download counters, and bounded streaming reassembly.
- Forge Generic Artifacts: Hierarchical namespace categorization (`<namespace>/<artifact>/<version>`), immutable versioning, and download streaming.
- Native Cargo Registry Support: Official RFC 2789 Cargo sparse index layout (`1`, `2`, `3/a`, `sp/in`), `config.json` endpoint, binary publish wire protocol unpacker (4-byte LE JSON length + metadata + 4-byte LE crate length + `.crate` tarball), SHA-256 crate checksums (`cksum`), download streaming, and version yanking/unyanking.
- Developer Experience & UI: Live `cargo publish` verified with official Cargo CLI; React 18 frontend expanded with Releases and Package Registry tabs, binary download triggers, and storage quota progress bar.
- Windows 11 Native Stability: Bounded memory footprints (< 12 MiB per streaming operation) preventing system paging exhaustion, atomic temporary file replacement, and 100% test pass rate across all crates.

### Batch 12: Forge Automation Platform, Webhooks, CI/CD Workflows, and Isolated Build Runners
- Native Workflow Engine (`spine-workflow`): Declarative `.spine/workflows/*.yml` YAML/JSON parser with line-number diagnostics, trigger filter evaluation (`push.branches`, `pull_request.branches`, `workflow_dispatch`), and Directed Acyclic Graph (`JobDag`) with Tarjan cycle detection and topological sorting.
- Forge CI/CD Orchestration: Control plane service (`WorkflowService`), scheduler (`WorkflowScheduler`), immutable `WorkflowRun` instances, multi-job DAG execution, step tracking, and PR check status reporting (`workflow_check_results`).
- Authoritative Branch Protection Gatekeeper: `check_merge_allowed` blocks PR merges unless all configured `required_status_checks` report `status: "SUCCESS"` for the target commit.
- Dedicated Isolated Runner Architecture: Strict control plane vs runner execution plane isolation boundary. `RunnerService` with CSPRNG registration tokens, heartbeat tracking, atomic job acquisition, fenced leases with monotonically incrementing tokens, and stale token rejection.
- Isolated Sandboxed Runner Executor (`IsolatedRunnerExecutor`): Ephemeral workspace lifecycle (`runner_workspaces/<job_id>`), environment sanitization, hard step timeouts, process-tree termination (`taskkill /PID ... /T /F` on Windows), and realtime log streaming with dynamic secret masking (`***`).
- Webhook Subsystem & Transactional Outbox: `WebhookService`, `WebhookOutboxWorker`, RFC 2104 HMAC-SHA256 signatures (`X-Spine-Signature-256`), constant-time signature verification, exponential backoff retries, and delivery logging.
- Strict SSRF Defenses (`SsrfValidator`): Scheme & port filtering, blocking loopback, RFC 1918 private, link-local / cloud metadata (`169.254.169.254`), broadcast, and multicast ranges, with DNS pinning preventing TOCTOU DNS rebinding attacks.
- Frontend Collaboration Suite: React 18 / TypeScript frontend expanded with dedicated Actions (DAG view, live console logs), Webhooks (subscriptions, test deliveries), Runners (connected agents, concurrency, registration), and Secrets management tabs.
- Full Windows 11 Native Integration: Zero mock compromises, verified against native Windows MSVC toolchains and official CLI workflows.

---

## Next Milestones

### Batch 13: Search, Code Discovery, Discussions, Security Center, and Developer Discovery
- Trigram and AST-aware code search, symbol indexing, and file path filtering.
- Forge Discussions with category channels, threaded answers, mark as answer, and reactions.
- Security Center with dependency vulnerability tracking, automated SBOM generation (SPDX / CycloneDX), and pre-receive secret scanning.
- Developer discovery, explore feeds, and trending repositories.

### Batch 14: Full-System Integration, Security Hardening, Reliability, Recovery, and Performance
- End-to-end integration across native VCS, cluster nodes, and Forge web endpoints.
- Transactional reconciliation, crash consistency checks, and failover validation.
- Hardened rate-limiting, authentication barriers, and audit trail validation.

### Batch 15: Native Git Interoperability, Repository Import/Export, and Platform Migration
- Bidirectional Git importer and exporter (`spine-migration`).
- Full round-trip preservation of commit history, tree graphs, tags, author metadata, and branch topologies.
- Archive sanitization, zip bomb defenses, and path traversal mitigation for platform migrations.

### Batch 16: Native Git Smart HTTP, Remote Operations, and Transport Integration
- Pure-Rust Git Smart HTTP protocol support (`git-upload-pack`, `git-receive-pack`).
- Streaming pkt-line protocol parser and packfile generation.
- Full compatibility with official Git CLI (`git clone`, `git push`, `git fetch`) without external Git or libgit2 dependencies.
- Authoritative CAS push verification, branch protection enforcement, and secret scanning on HTTP pushes.

### Batch 17: Native Git LFS HTTP API, Smart HTTP Hardening, and Large-File Remote Compatibility
- Official Git LFS v1 Batch API and Basic Transfer Adapter.
- Bidirectional identity mapping between Git LFS SHA-256 OIDs and `spine-artifact` BLAKE3 manifests.
- Pre-receive LFS pointer integrity fencing preventing pushes with un-uploaded binary blobs.
- Streaming verification and HMAC capability token security.

### Batch 18: Native Git-over-SSH, Key Management, Git LFS Auth, and File Locking
- Pure-Rust native SSH daemon (`russh 0.65`, `ssh-key 0.7`) with Ed25519 host key generation and zero shell access.
- User SSH public key management and repository deploy keys with read-only vs write-enabled enforcement.
- Git LFS SSH authentication (`git-lfs-authenticate`) issuing scoped `RemoteAuth` HMAC tokens for HTTPS transfer.
- Standard Git LFS File Locking API (`locks`, `verify`, `unlock`) with pre-receive lock conflict checks in HTTP and SSH receive-pack pipelines.
- Modern React/TypeScript frontend controls for SSH keys, deploy keys, clone protocol switching, and LFS locks.

### Batch 19: Production Packaging, PostgreSQL Readiness, Deployment Architecture, Backup/Recovery, and Operational Rehearsal
- System inventory across 35 subsystems and deployment blocker register.
- Strict host key checking verification with official OpenSSH 9.5p2 client (`StrictHostKeyChecking=yes`, isolated `known_hosts`).
- SSH host key persistence across server restarts and public key revocation enforcement.
- 11 sequential, idempotent PostgreSQL migration files (`0001` to `0011`) covering 60+ tables and embedded transactional runner.
- Coordinated backup and disaster recovery subsystem (`spine-api backup create/restore`) generating and verifying SHA-256 `.tar.gz` archives.
- One-time cryptographically secure admin bootstrap CLI (`spine-api bootstrap-admin`) with single-execution lockout and audit logging.
- Hardened multi-stage container packaging (`Dockerfile.production`, `docker-compose.yml`, `docker-compose.dev.yml`) with unprivileged `spine` user (UID 10001).
- Hardened Linux systemd units (`spine-api.service`, `spine-coordinator.service`, `spine-storage-node.service`).
- Comprehensive operations documentation, incident runbooks, storage economics model (200 GB+ dataset), and final deployment audit handoff.

---

## Future Milestones

### Batch 20: Enterprise Federation, SAML/OIDC, Multi-Region Replication, and High-Availability Clustering
- Enterprise Single Sign-On (SAML 2.0 and OIDC).
- Multi-region distributed storage replication and geo-distributed caching mirrors.
- Real-time collaborative review feeds with WebSockets.
