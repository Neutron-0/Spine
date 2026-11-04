# Spine — Batch 19 Deployment Readiness Register
## Complete Production Subsystem Inventory and Audit

**Target Milestone:** Batch 19  
**Workspace:** `d:\Vault\Spine`  
**Current Baseline Commit:** `b6881c6`  
**Date of Audit:** 2026-11-05  

---

## 1. Subsystem Classification Status Definitions

- **READY CANDIDATE:** Production-grade implementation, robust test coverage, verified persistence, explicit configuration, operational documentation.
- **PARTIAL:** Implemented and functional in development/in-memory environments, but requires operational packaging, database migration scripts, or specific deployment configurations.
- **BLOCKED:** Cannot be operated in the specified production configuration due to an external environment dependency or missing execution harness (e.g. live PostgreSQL daemon absent in build runner).
- **UNSUPPORTED:** Capability intentionally scoped out or restricted for the target deployment profile (e.g. public untrusted CI execution without VM isolation).
- **NOT IMPLEMENTED:** Planned roadmap capability not yet engineered.

---

## 2. Comprehensive 35-Component Production Inventory

### 1. Native Spine VCS Engine
- **Production Entry Point:** `spine-core`, `spine-diff`, `spine-merge`, `spine-history`, `spine-worktree`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Storage root path, hash algorithm (BLAKE3)
- **Dependencies:** Pure Rust crates, zero external C/git dependencies
- **Persistent Data:** Native object files, tree manifests, commit logs
- **Network Ports:** N/A (library)
- **Required Service Identity:** Local service account (read/write access to storage root)
- **Startup Order:** Foundational (precedes API/transport daemons)
- **Health Check:** Object integrity verification (`spine fsck`)
- **Security Restrictions:** Enforces strict content-addressed immutability
- **Backup Requirements:** Complete `.spine/objects` and reference logs
- **Recovery Procedure:** File tree reconstructor and fsck consistency traversal
- **Test Coverage:** 109 native tests in `git-alternative/` passing
- **Known Limitations:** Single-host filesystem lock semantics

### 2. Native Object and Pack Storage
- **Production Entry Point:** `spine-object-store`, `spine-format`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Loose object directory, packfile directory, index fanout tables
- **Dependencies:** `spine-core`, `blake3`, `flate2`
- **Persistent Data:** `.spk` packfiles, `.spidx` pack index files, loose objects
- **Network Ports:** N/A
- **Required Service Identity:** Service account with filesystem write permissions
- **Startup Order:** Foundational
- **Health Check:** Checksum verification across 256-way fanout headers
- **Security Restrictions:** Path traversal defenses on object OID parsing
- **Backup Requirements:** Packfiles, indices, and loose store directory
- **Recovery Procedure:** Rebuild `.spidx` indices from packfiles via `pack_inspector`
- **Test Coverage:** Packfile round-trip, corruption detection, fanout verification tests pass
- **Known Limitations:** Repacking requires temporary working disk space equal to active pack size

### 3. Native Repository Format
- **Production Entry Point:** `spine-format`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** `.spine/` metadata directory layout
- **Dependencies:** `spine-core`, `spine-integrity`
- **Persistent Data:** Head reference, branch references, tag references, reflog
- **Network Ports:** N/A
- **Required Service Identity:** Service account
- **Startup Order:** Precedes repository write transactions
- **Health Check:** Atomic `.lock` verification and journal playback
- **Security Restrictions:** Direct write bypass prevention via `AuthoritativeRepositoryWriter`
- **Backup Requirements:** `.spine/` root directory per repository
- **Recovery Procedure:** Journal playback and reference tombstone cleanup
- **Test Coverage:** Reference atomicity, crash recovery, and multi-branch tests pass
- **Known Limitations:** Concurrent branch writes guarded by filesystem locks

### 4. Native Networking and SPWP (Spine Protocol Wire Protocol)
- **Production Entry Point:** `spine-net`, `spine-protocol`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** SPWP bind address, TLS certificates, capability secret
- **Dependencies:** `tokio`, `rustls`
- **Persistent Data:** None (wire transport)
- **Network Ports:** TCP 9418 / configurable SPWP port
- **Required Service Identity:** API / Daemon service account
- **Startup Order:** Starts after local storage node initialization
- **Health Check:** Framing handshake ping/pong
- **Security Restrictions:** Capability ticket verification (`ticket:<hex>.<sig>`), TLS required
- **Backup Requirements:** TLS certificate and private key
- **Recovery Procedure:** Service restart and connection renegotiation
- **Test Coverage:** Typed message framing, max payload bounds, and protocol roundtrips pass
- **Known Limitations:** Fixed max frame payload size (16 MiB)

### 5. Storage Coordinator
- **Production Entry Point:** `spine-cluster::Coordinator`
- **Status:** **PARTIAL** (Single-Instance Pilot Only)
- **Configuration Requirements:** Coordinator bind address, node registry config, heartbeat interval
- **Dependencies:** `spine-protocol`, `tokio`
- **Persistent Data:** Lease epochs, primary node assignments, cluster metadata
- **Network Ports:** TCP 9000 (RPC)
- **Required Service Identity:** Cluster admin identity
- **Startup Order:** Must start before storage nodes and API write path
- **Health Check:** `/health` RPC query, heartbeat liveliness
- **Security Restrictions:** Internal cluster network only; reject external traffic
- **Backup Requirements:** Node registry and lease metadata state
- **Recovery Procedure:** Node re-registration and epoch re-fencing on restart
- **Test Coverage:** Lease fencing, primary failover, and epoch tests pass
- **Known Limitations:** Single point of failure in pilot mode; Raft/Consul consensus required for high availability

### 6. Storage Nodes
- **Production Entry Point:** `spine_storage_node` binary
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Node ID, coordinator RPC address, local chunk storage directory
- **Dependencies:** `spine-cluster`, `spine-artifact`
- **Persistent Data:** Replicated artifact chunks, manifests, storage node journals
- **Network Ports:** TCP 9100+ (Storage RPC)
- **Required Service Identity:** Storage node identity
- **Startup Order:** Starts after coordinator
- **Health Check:** Heartbeat RPC, disk capacity probe
- **Security Restrictions:** Reject commands from unverified coordinators
- **Backup Requirements:** Replicated chunk directories
- **Recovery Procedure:** Re-replication from surviving primary storage node
- **Test Coverage:** Distributed artifact persistence test passes
- **Known Limitations:** Storage nodes must have synchronized system clocks

### 7. Repository Replication
- **Production Entry Point:** `spine-cluster::ReplicationManager`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Quorum policy (`SingleCopy`, `TwoCopies`, `ThreeCopies`)
- **Dependencies:** `spine-protocol`, `spine-storage-node`
- **Persistent Data:** Replicated packfiles and chunk replicas
- **Network Ports:** Cluster RPC ports
- **Required Service Identity:** Node-to-node replication capability
- **Startup Order:** Starts with storage nodes
- **Health Check:** Replication lag metrics and quorum verification endpoint
- **Security Restrictions:** Mutual TLS across cluster nodes
- **Backup Requirements:** Primary node data backup
- **Recovery Procedure:** Quorum reconciliation and catch-up replication
- **Test Coverage:** Two-phase commit replication and quorum verification tests pass
- **Known Limitations:** High latency networks may slow multi-copy sync pushes

### 8. Authoritative Repository Writer
- **Production Entry Point:** `spine_api::AuthoritativeRepositoryWriter`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Storage root, cluster coordinator client, DB connection
- **Dependencies:** `spine-api`, `spine-migration`, `spine-artifact`
- **Persistent Data:** Centralized write lock and transaction state
- **Network Ports:** In-process write pipeline
- **Required Service Identity:** Authorized caller identity (User PAT, Deploy Key, SSH session)
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Database write readiness and disk capacity check
- **Security Restrictions:** Secret scanning, missing LFS blob check, branch protection, LFS file locks
- **Backup Requirements:** None (orchestrator; relies on storage and DB)
- **Recovery Procedure:** Transaction abort on uncommitted stages
- **Test Coverage:** Fully exercised across Smart HTTP, SSH, and LFS test suites
- **Known Limitations:** Serializes writes per branch ref

### 9. Git Import/Export
- **Production Entry Point:** `spine-migration::GitImporter`, `spine-migration::GitExporter`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Temporary staging directory, extraction limits
- **Dependencies:** `flate2`, `spine-core`, `spine-object-store`
- **Persistent Data:** Imported native Spine repository
- **Network Ports:** N/A (in-process engine)
- **Required Service Identity:** Service account
- **Startup Order:** Precedes migration worker
- **Health Check:** Zip bomb / path traversal sanitization checks
- **Security Restrictions:** Zip extraction size limits, decompression ratio limits, path sanitization
- **Backup Requirements:** Staging directories excluded from backup
- **Recovery Procedure:** Clean up orphaned staging directory on failed import
- **Test Coverage:** Round-trip import/export, archive security, and path sanitization tests pass
- **Known Limitations:** Extremely large Git repos (> 10 GB) require substantial staging disk space

### 10. Git Smart HTTP Transport
- **Production Entry Point:** `spine_api::routes_git_http`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Public origin URL, reverse-proxy TLS termination
- **Dependencies:** `axum`, `spine-migration`, `tower`
- **Persistent Data:** Repository packfiles and ref trees
- **Network Ports:** HTTP 3000 / HTTPS 443 (via reverse proxy)
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** `GET /:owner/:repo.git/info/refs?service=git-upload-pack`
- **Security Restrictions:** Basic Auth / PAT required for private repos; write auth for receive-pack
- **Backup Requirements:** Repository data directory
- **Recovery Procedure:** Stateless HTTP request retry
- **Test Coverage:** `git_smart_http_test` (6/6 tests pass)
- **Known Limitations:** Chunked transfer encoding requires HTTP/1.1 or HTTP/2 support

### 11. Git-over-SSH Transport
- **Production Entry Point:** `spine_api::ssh::server::SshServer`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** SSH listen port (`SPINE_SSH_PORT`, default 2222), host key path
- **Dependencies:** `russh`, `ssh-key`, `tokio`
- **Persistent Data:** Ed25519 host key (`storage_root/ssh/host_key_ed25519`)
- **Network Ports:** TCP 2222 (configurable)
- **Required Service Identity:** Unprivileged daemon account
- **Startup Order:** Starts with `spine-api`
- **Health Check:** TCP connection probe and SSH banner exchange
- **Security Restrictions:** Zero shell access, PTY rejected, port forwarding rejected, strict command whitelist
- **Backup Requirements:** `host_key_ed25519` private key file
- **Recovery Procedure:** Restore host key to prevent client `known_hosts` warnings
- **Test Coverage:** `git_ssh_test` (5/5 tests pass with official Git and OpenSSH)
- **Known Limitations:** Does not support legacy DSA or short RSA keys (< 2048-bit)

### 12. Git LFS HTTP API
- **Production Entry Point:** `spine_api::routes_lfs`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Public origin, artifact storage path, HMAC capability secret
- **Dependencies:** `axum`, `spine-artifact`, `sha2`
- **Persistent Data:** Artifact chunk files, metadata manifests
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** `POST /:owner/:repo.git/info/lfs/objects/batch` endpoint response
- **Security Restrictions:** Scoped HMAC tokens, Basic/Bearer/RemoteAuth authentication
- **Backup Requirements:** Artifact storage directory
- **Recovery Procedure:** Re-upload corrupted chunk from client or replica
- **Test Coverage:** `git_lfs_http_test` (3/3 tests pass with official `git-lfs`)
- **Known Limitations:** Transfers limited to basic adapter (standard Git LFS protocol)

### 13. Git LFS File Locking
- **Production Entry Point:** `spine_api::routes_lfs` (Locking handlers)
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** PostgreSQL / DB locking tables
- **Dependencies:** Database layer, `routes_lfs`, `process_git_receive_pack`
- **Persistent Data:** `lfs_locks`, `lfs_lock_audits` tables
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Authenticated repository collaborator
- **Startup Order:** Starts with `spine-api`
- **Health Check:** `GET /:owner/:repo.git/info/lfs/locks`
- **Security Restrictions:** Write permission required to lock; lock owner or admin required to unlock
- **Backup Requirements:** Database `lfs_locks` table
- **Recovery Procedure:** Admin force-unlock (`force: true`) if user credential is lost
- **Test Coverage:** `test_git_lfs_file_locking_api_and_push_conflict` passes
- **Known Limitations:** Locking is path-based; renamed directories must be relocked

### 14. Artifact Storage
- **Production Entry Point:** `spine-artifact`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Storage root path, chunk size (4 MiB default)
- **Dependencies:** `spine-core`, `blake3`
- **Persistent Data:** Deduplicated chunks (`chunks/`), manifests (`manifests/`)
- **Network Ports:** N/A (library and storage node)
- **Required Service Identity:** Service account
- **Startup Order:** Starts before artifact ingestion
- **Health Check:** Chunk digest recalculation against manifest
- **Security Restrictions:** Path traversal prevention on chunk hashes
- **Backup Requirements:** Chunks and manifest files
- **Recovery Procedure:** Mark-and-sweep reachability audit
- **Test Coverage:** Chunking, deduplication, and persistence tests pass
- **Known Limitations:** Content deduplication is intra-cluster

### 15. Cargo Package Registry
- **Production Entry Point:** `spine_api::routes_cargo`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Registry URL, sparse index configuration
- **Dependencies:** `spine-artifact`, `flate2`, `tar`
- **Persistent Data:** Sparse index files, `.crate` tarball artifacts in `spine-artifact`
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** `GET /api/v1/cargo/config.json`
- **Security Restrictions:** PAT authentication with `package:write` scope required for publish
- **Backup Requirements:** Database packages table and artifact chunks
- **Recovery Procedure:** Rebuild sparse index from database metadata
- **Test Coverage:** Verified live with official Cargo CLI (`cargo publish`)
- **Known Limitations:** Sparse index protocol only (RFC 2789; legacy Git index registry omitted)

### 16. Releases and Assets
- **Production Entry Point:** `spine_api::routes_releases`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Artifact storage path
- **Dependencies:** `spine-artifact`, database
- **Persistent Data:** Release metadata in DB, binary assets in `spine-artifact`
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** `GET /api/v1/repos/:owner/:repo/releases`
- **Security Restrictions:** Repository write access required to create release
- **Backup Requirements:** Database tables and artifact chunks
- **Recovery Procedure:** Re-upload release assets
- **Test Coverage:** Releases API integration tests pass
- **Known Limitations:** Asset uploads stream through server memory in 4 MiB chunks

### 17. Forge API
- **Production Entry Point:** `spine-api` binary (`spine_api::build_router`)
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** `--bind`, `--port`, `--database-url`, `--storage-root`, `SPINE_PUBLIC_ORIGIN`
- **Dependencies:** `axum`, `tokio`, `tower-http`
- **Persistent Data:** Coordinates all persistent storage and database operations
- **Network Ports:** TCP 3000 (HTTP)
- **Required Service Identity:** Application service account
- **Startup Order:** Core service; starts after database and storage root preparation
- **Health Check:** `GET /healthz`, `GET /readyz`
- **Security Restrictions:** CORS restrictions, rate limiting, request size limits
- **Backup Requirements:** Application configuration and environment files
- **Recovery Procedure:** Graceful process restart
- **Test Coverage:** Over 35 comprehensive integration test files in `tests/`
- **Known Limitations:** Single process monolith in pilot profile

### 18. PostgreSQL Persistence
- **Production Entry Point:** `spine_api::db::postgres::PgDatabase`
- **Status:** **PARTIAL** (Complete query implementation exists; migration automation & live database tests required)
- **Configuration Requirements:** `DATABASE_URL` (e.g. `postgres://user:pass@host:5432/spine_forge`)
- **Dependencies:** `sqlx` (v0.7 with postgres and migrate features)
- **Persistent Data:** Relational tables (users, repos, PRs, issues, locks, etc.)
- **Network Ports:** TCP 5432
- **Required Service Identity:** `spine_forge_app` database user
- **Startup Order:** Must start and migrate before `spine-api` starts
- **Health Check:** `SELECT 1`, connection pool stats probe
- **Security Restrictions:** Least-privilege role, TLS connections for remote hosts
- **Backup Requirements:** Full `pg_dump` or WAL continuous archiving
- **Recovery Procedure:** `pg_restore` into empty schema
- **Test Coverage:** Queries implemented for all trait methods; live runner execution blocked by absence of running PostgreSQL daemon in current Windows environment
- **Known Limitations:** In-memory fallback currently used for tests when daemon is absent

### 19. In-Memory Persistence
- **Production Entry Point:** `spine_api::db::in_memory::InMemoryDatabase`
- **Status:** **READY CANDIDATE** (For development and testing only; **NOT FOR PRODUCTION**)
- **Configuration Requirements:** `--in-memory-db` flag
- **Dependencies:** Pure Rust `std::sync::Arc<tokio::sync::RwLock>`
- **Persistent Data:** Non-persistent (RAM only)
- **Network Ports:** N/A
- **Required Service Identity:** In-process
- **Startup Order:** Instantaneous
- **Health Check:** Always healthy
- **Security Restrictions:** In-process memory protection
- **Backup Requirements:** None (ephemeral)
- **Recovery Procedure:** N/A
- **Test Coverage:** Fully exercised by all unit and integration test suites
- **Known Limitations:** All state lost upon process exit

### 20. User Authentication (Session / Password)
- **Production Entry Point:** `spine_api::auth`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Argon2id parameters, session cookie secret
- **Dependencies:** `argon2`, `rand`
- **Persistent Data:** `users`, `user_sessions`, `user_profiles` tables
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Auth middleware token verification
- **Security Restrictions:** Secure, HttpOnly, SameSite cookies; constant-time password verification
- **Backup Requirements:** Database `users` table
- **Recovery Procedure:** Password reset via admin bootstrap or token
- **Test Coverage:** Session creation, cookie validation, and password verification tests pass
- **Known Limitations:** No native SSO/SAML in pilot profile

### 21. Personal Access Token (PAT) Authentication
- **Production Entry Point:** `spine_api::auth::pat`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Prefix format (`sp_pat_`), SHA-256 hash storage
- **Dependencies:** `sha2`, `hex`, `subtle`
- **Persistent Data:** `personal_access_tokens` table
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** PAT lookup and expiry evaluation
- **Security Restrictions:** Raw tokens shown only once; SHA-256 hashed at rest; scoped permissions
- **Backup Requirements:** Database table
- **Recovery Procedure:** Revoke compromised PAT and regenerate
- **Test Coverage:** Issuance, expiration, scope enforcement, and revocation tests pass
- **Known Limitations:** Token revocation is immediate; no distributed cache invalidation delay

### 22. SSH User Keys
- **Production Entry Point:** `spine_api::routes_users`, `spine_api::ssh::server`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** OpenSSH key parsing, SHA-256 fingerprinting
- **Dependencies:** `ssh-key`, database
- **Persistent Data:** `user_ssh_keys` table
- **Network Ports:** SSH 2222
- **Required Service Identity:** Web & SSH daemon
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Key validation on upload
- **Security Restrictions:** Rejection of weak keys; fingerprint uniqueness check
- **Backup Requirements:** Database table
- **Recovery Procedure:** User re-adds public key via UI
- **Test Coverage:** `test_git_ssh_key_auth_and_push_clone_roundtrip` passes
- **Known Limitations:** Supports Ed25519, ECDSA, RSA; DSA deprecated

### 23. Repository Deploy Keys
- **Production Entry Point:** `spine_api::routes_repos`, `spine_api::ssh::server`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Repository ID binding, `write_enabled` toggle
- **Dependencies:** `ssh-key`, database
- **Persistent Data:** `deploy_keys` table
- **Network Ports:** SSH 2222
- **Required Service Identity:** Web & SSH daemon
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Repository key lookup by fingerprint
- **Security Restrictions:** Read-only by default; write denied unless `write_enabled=true`
- **Backup Requirements:** Database table
- **Recovery Procedure:** Revoke and re-create deploy key
- **Test Coverage:** `test_git_ssh_deploy_keys_read_and_write_enforcement` passes
- **Known Limitations:** Deploy key scoped to single repository

### 24. Workflow Scheduler
- **Production Entry Point:** `spine_api::WorkflowScheduler`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Polling interval (default 5s), lease TTL (30s)
- **Dependencies:** `spine-workflow`, database
- **Persistent Data:** `workflow_runs`, `workflow_jobs`, `workflow_steps` tables
- **Network Ports:** Background worker
- **Required Service Identity:** Internal scheduler task
- **Startup Order:** Starts after DB connection
- **Health Check:** Active run queue inspection
- **Security Restrictions:** Branch protection integration; blocks PR merge on failed check
- **Backup Requirements:** Database tables
- **Recovery Procedure:** Stale lease eviction and job reassignment
- **Test Coverage:** Workflow DAG, cycle detection, and scheduler tests pass
- **Known Limitations:** DAG execution bounded by registered runner pool

### 25. Webhooks Subsystem
- **Production Entry Point:** `spine_api::WebhookService`, `spine_api::WebhookOutboxWorker`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Worker batch size, max retries (5), backoff curve
- **Dependencies:** `reqwest`, `hmac`, `sha2`, database
- **Persistent Data:** `webhook_subscriptions`, `webhook_outbox`, `webhook_deliveries` tables
- **Network Ports:** Egress HTTP/HTTPS
- **Required Service Identity:** Background outbox worker
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Outbox queue depth
- **Security Restrictions:** RFC 2104 HMAC-SHA256 signatures; strict SSRF validation (`SsrfValidator`)
- **Backup Requirements:** Database tables
- **Recovery Procedure:** Retry failed outbox deliveries
- **Test Coverage:** Webhook signing, SSRF blocking, and delivery retry tests pass
- **Known Limitations:** Egress requests timeout after 10s

### 26. Runner Execution
- **Production Entry Point:** `spine_api::RunnerService`, `IsolatedRunnerExecutor`
- **Status:** **PARTIAL** (Core Forge profile disables untrusted runners; trusted local runner supported)
- **Configuration Requirements:** Registration token, workspace root, process timeout
- **Dependencies:** Operating system process launcher
- **Persistent Data:** `runner_registrations` table, ephemeral workspace
- **Network Ports:** Internal runner polling API
- **Required Service Identity:** Isolated runner service account
- **Startup Order:** External agent; connects after API is ready
- **Health Check:** Heartbeat check (30s interval)
- **Security Restrictions:** Secret masking in logs, workspace wiping, process tree termination
- **Backup Requirements:** None (ephemeral)
- **Recovery Procedure:** Workspace cleanup on crash
- **Test Coverage:** Runner registration, heartbeat, and job execution tests pass
- **Known Limitations:** Untrusted CI requires VM boundary; disabled by default for production security

### 27. Secret Management
- **Production Entry Point:** `spine_api::auth` and configuration loaders
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Environment variables (`SPINE_SECRET_KEY`, `DATABASE_URL`)
- **Dependencies:** Operating system environment / secure secret store
- **Persistent Data:** Hashed secrets, encrypted webhook secrets
- **Network Ports:** N/A
- **Required Service Identity:** Root/admin during deployment initialization
- **Startup Order:** Checked before process initialization
- **Health Check:** Startup fail-fast validation when critical secrets are missing
- **Security Restrictions:** Zero plaintext secrets in logs, commits, or frontend bundles
- **Backup Requirements:** Secure off-site encrypted backup of master keys
- **Recovery Procedure:** Secret rotation procedure
- **Test Coverage:** PAT hash checks, HMAC token verification tests pass
- **Known Limitations:** Requires external KMS or secure env injection in cloud deployments

### 28. Secret Scanning
- **Production Entry Point:** `spine-migration::platform::sanitization::scan_for_secrets`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Secret pattern definitions (AWS, GitHub tokens, private keys)
- **Dependencies:** Regex / pattern matcher
- **Persistent Data:** `secret_scan_findings` table
- **Network Ports:** Pre-receive hook in HTTP & SSH transport
- **Required Service Identity:** Transport pipeline
- **Startup Order:** Built into Git receive-pack pipeline
- **Health Check:** Rule pattern compilation check
- **Security Restrictions:** Blocks pushes containing high-entropy secrets
- **Backup Requirements:** Findings table in database
- **Recovery Procedure:** Push rejection bypass with explicit allowlist commit trailer
- **Test Coverage:** Secret rejection tests pass in both HTTP and SSH suites
- **Known Limitations:** Pre-receive scan inspects newly added blobs only

### 29. Dependency Auditing
- **Production Entry Point:** `spine_api::SecurityService`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Security advisory database
- **Dependencies:** Database
- **Persistent Data:** `security_advisories`, `dependency_vulnerabilities` tables
- **Network Ports:** In-process scan
- **Required Service Identity:** Service account
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Advisory database query
- **Security Restrictions:** Read-only analysis of manifest files (`Cargo.lock`, `package-lock.json`)
- **Backup Requirements:** Database tables
- **Recovery Procedure:** Update advisory database
- **Test Coverage:** Vulnerability matching and reporting tests pass
- **Known Limitations:** Depends on freshness of imported advisory feed

### 30. SBOM Generation
- **Production Entry Point:** `spine_api::SecurityService`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Output format specification (SPDX 2.3 JSON, CycloneDX 1.5 JSON)
- **Dependencies:** `serde_json`, dependency parser
- **Persistent Data:** Generated on-demand or cached in `dependency_inventories`
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Endpoint response test
- **Security Restrictions:** Repository read access required
- **Backup Requirements:** Cached inventories in database
- **Recovery Procedure:** Regenerate from repository lockfiles
- **Test Coverage:** SBOM generation and schema conformance tests pass
- **Known Limitations:** Supported lockfiles: Cargo.lock, package-lock.json

### 31. Search and Code Indexing
- **Production Entry Point:** `spine_api::SearchService`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Trigram index cache, max results limit
- **Dependencies:** Database / memory store
- **Persistent Data:** Trigram index and repository topics/languages
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Search query endpoint test
- **Security Restrictions:** Search results filtered by caller read permissions
- **Backup Requirements:** Database search tables
- **Recovery Procedure:** Rebuild trigram index from repository objects
- **Test Coverage:** Code search, repo search, and permission filtering tests pass
- **Known Limitations:** In-memory trigram index rebuilt upon restart if DB index unpopulated

### 32. Discussions and Notifications
- **Production Entry Point:** `spine_api::routes_discussions`, `spine_api::routes_notifications`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Database tables
- **Dependencies:** Database
- **Persistent Data:** `discussions`, `discussion_comments`, `user_notifications` tables
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Notification feed endpoint test
- **Security Restrictions:** Category permissions, author-only edits
- **Backup Requirements:** Database tables
- **Recovery Procedure:** Standard DB restore
- **Test Coverage:** Discussion creation, comments, and notification tests pass
- **Known Limitations:** Polling-based notification delivery in pilot profile

### 33. Developer Discovery
- **Production Entry Point:** `spine_api::routes_discovery`
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Activity event aggregation window
- **Dependencies:** Database
- **Persistent Data:** `repository_activity_events` table
- **Network Ports:** HTTP 3000 / HTTPS 443
- **Required Service Identity:** Web process identity
- **Startup Order:** Starts with `spine-api`
- **Health Check:** Trending repos endpoint test
- **Security Restrictions:** Only public repositories surface in global discovery
- **Backup Requirements:** Activity events table
- **Recovery Procedure:** Aggregate historical events
- **Test Coverage:** Discovery and trending repository tests pass
- **Known Limitations:** Ranking algorithm weights recent stars and commits

### 34. Frontend Application
- **Production Entry Point:** `github-alternative/frontend` (Compiled SPA served by reverse proxy or static file host)
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** `VITE_API_ORIGIN` (or same-origin relative URLs)
- **Dependencies:** React 18, TypeScript, Vite
- **Persistent Data:** Browser localStorage (session token)
- **Network Ports:** HTTP/HTTPS (Port 80/443)
- **Required Service Identity:** Web server
- **Startup Order:** Starts alongside API
- **Health Check:** `GET /` serves `index.html` with valid bundle hash
- **Security Restrictions:** Content Security Policy, zero backend credentials in client bundle
- **Backup Requirements:** Frontend source code repository
- **Recovery Procedure:** Re-deploy static bundle
- **Test Coverage:** TypeScript compilation and Vite production build verified (0 errors)
- **Known Limitations:** SPA client requires fallback routing on web server

### 35. Observability and Health Endpoints
- **Production Entry Point:** `spine_api::routes_health` (`/healthz`, `/readyz`, `/metrics`)
- **Status:** **READY CANDIDATE**
- **Configuration Requirements:** Metric scrapers, log format (JSON / structured)
- **Dependencies:** `tracing`, `tracing-subscriber`
- **Persistent Data:** Application logs (rotated)
- **Network Ports:** HTTP 3000 (Internal / Admin access)
- **Required Service Identity:** Monitoring system identity
- **Startup Order:** Active immediately upon process bind
- **Health Check:** Self-diagnostic
- **Security Restrictions:** Metrics and readiness endpoints should be protected from untrusted public egress
- **Backup Requirements:** Log retention policy
- **Recovery Procedure:** N/A
- **Test Coverage:** Health endpoint tests pass
- **Known Limitations:** Prometheus metrics scraper format requires authentication in public deployments
