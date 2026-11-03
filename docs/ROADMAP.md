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

---

## Next Milestones

### Batch 11: Real-time Collaboration, Webhooks, CI/CD Pipeline Gateway, and Large Object Storage (LFS)
- Real-time updates via WebSockets for collaborative review and notification feeds.
- Webhook dispatch system with HMAC-SHA256 payload signing and retry queues.
- Spine LFS: Content-addressed external binary asset store with chunked streaming and content-hash verification.
- Partial and sparse worktree support for monorepo development.

### Batch 12: Enterprise Audit Logging, SAML/OIDC SSO, and Global Federation
- Enterprise Single Sign-On (SAML 2.0 and OIDC).
- Comprehensive immutable audit log exports and compliance reporting.
- Global repository replication and geo-distributed caching mirrors.
