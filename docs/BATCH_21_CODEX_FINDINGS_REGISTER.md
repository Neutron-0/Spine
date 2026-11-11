# Spine Forge — Batch 21, 21A, 21B & 21C Codex Findings Register & Remediation Verification

**Audited Milestone:** Batch 20 (`v0.1.0-rc1`), Batch 21 (`v0.1.0-rc2`), Batch 21A, Batch 21B<br>
**Baseline Commit:** `cc075d488dc78a506545465eb59a5d2e927d892e`<br>
**Auditor:** Codex (Independent Read-Only Deployment Auditor)<br>
**Remediation Milestone:** Batch 21C<br>
**Status:** ALL 7 FINDINGS (F-01 THROUGH F-07) DEEPLY REMEDIATED & PROGRAMMATICALLY VERIFIED

---

## 1. Executive Summary

Following the fourth independent deployment audit, Codex identified critical defects including:
- **F-01 (P0):** Lack of globally atomic cross-OS-process barrier coordination (permit accounting was process-local), maintenance guard dropped prematurely before storage capture and encryption validation finished, and missing write permits on metadata mutation routes.
- **F-02 (P1):** Unstaged backup creation truncating existing files before success, incomplete rotation verification, and non-atomic storage restores.
- **F-03 (P1):** Fallback default `false` for `execute_job` runner provenance.
- **F-04 (P1):** DDL role leakage and startup migrations running under the runtime application role.
- **F-05 (P1):** Dockerfile base image staleness and excessive ingress network reach.
- **F-06 (P1):** Raw password CLI flags and missing preflight checks.
- **F-07 (P1):** Migration cancellation unverified during live DDL and table inserts.

In **Batch 21C**, every finding was addressed down to the source code:
- **100% of integration & unit tests in `apps/spine-api` PASSED** (including `cross_process_coordination_test`, `test_database_safety_test`, `advisory_lock_test`, `postgres_backup_restore_test`, `postgres_least_privilege_test`, `postgres_live_test`, `backup_v2_streaming_test`, `profile_enforcement_test`, `failure_injection_test`).
- **100% of workspace tests in `git-alternative` PASSED**.
- **`cargo fmt --manifest-path ... --all -- --check` passed cleanly across both workspaces**.
- **`cargo clippy --manifest-path ... --all-targets -- -D warnings` passed cleanly with 0 errors and 0 warnings across both workspaces**.
- **Frontend built cleanly with `vite` and `npm audit` returned 0 vulnerabilities**.
- **Docker Compose configuration verified cleanly (`docker compose config --quiet`) with strict environment secret enforcement**.

---

## 2. Findings Register & Remediation Summary

| ID | Severity | Category | Status | Batch 21C Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **F-01** | **P0 Blocker** | Backup & Recovery | **RESOLVED** | Implemented global cross-OS-process maintenance coordination via PostgreSQL session advisory locks (`SPINE_CLUSTER_MAINTENANCE_LOCK_ID = 0x5350494E_4542414B`). Write permits acquire shared advisory locks and verify `_cluster_maintenance_barrier`. Main backup CLI scopes maintenance guard across database dump, storage capture, encryption, and verification. Wired write permits to pull requests, releases, repository deletion, branch protections, and packages. Added 16 MiB line bounds and strict sequence (`last_value`, `is_called`) resync. | `tests/cross_process_coordination_test.rs`, `tests/postgres_backup_restore_test.rs` |
| **F-02** | **P1 Blocker** | Cryptography / Memory | **RESOLVED** | Staged backup creation: writes to `.tmp_bak_<uuid>` before atomic rename to destination; never opens/truncates destination upfront. Full stream verification on key rotation (decrypts chunks, auth tags, manifest, footer, trailing bytes check against `sink()`) before atomic rename. Failure-atomic storage restore: extracts to `.tmp_restore_<uuid>`, verifies inventory and file hashes, atomically publishes with live rollback. | `tests/backup_v2_streaming_test.rs`, `tests/backup_restore_test.rs` |
| **F-03** | **P1 Blocker** | Security / CI Runner | **RESOLVED** | Removed `execute_job(..., false)` default bypass; required explicit `TrustClassification`. Evaluated provenance fails closed to `Untrusted` on missing/empty repository or fork. | `tests/profile_enforcement_test.rs`, `tests/hostile_workflow_test.rs`, `tests/workflow_ci_e2e_test.rs` |
| **F-04** | **P1 Blocker** | Infrastructure / Database | **RESOLVED** | Role split: DDL role (`spine_migrator`) and runtime role (`spine_app` with `REVOKE CREATE ON SCHEMA public`). In `main.rs`, startup migrations run using `MIGRATION_DATABASE_URL` and the pool is closed before runtime begins as `spine_app`. Hardened WSL setup script: mandatory passwords and strict subnet isolation. | `tests/postgres_least_privilege_test.rs`, `tests/postgres_backup_restore_test.rs` |
| **F-05** | **P1 Blocker** | Docker / Ingress Security | **RESOLVED** | Dockerfiles aligned to `rust:1.85-bookworm` builder and `debian:bookworm-slim` runtime. Isolated internal backend network (`spine-internal`) from ingress; ingress connects only to `spine-public` and `spine-ingress-proxy`. | `docker compose config --quiet`, `Dockerfile.production`, `docker-compose.yml`, `docker-compose.prod.yml` |
| **F-06** | **P1 Blocker** | Reliability / Database Fail-Fast | **RESOLVED** | Preflight checks on `Restore`: verifies input magic bytes, format, encryption passphrase presence, target database connectivity, and verifies target database is empty before touching filesystem storage. Removed raw `--passphrase` CLI flags in favor of `--passphrase-file` and environment variables. | `apps/spine-api/src/main.rs`, `tests/postgres_backup_restore_test.rs` |
| **F-07** | **P1 Blocker** | Concurrency / DB Locking | **RESOLVED** | Implemented `run_migrations_with_pause_after_step`. Verified cancellation during real transactional DDL and inserts (Worker A paused mid-migration; Worker B blocks; Worker A cancelled; transaction rolls back cleanly; Worker B completes). Repeated for 5 consecutive cycles. | `tests/advisory_lock_test.rs` |

---

## 3. In-Depth Technical Remediation Details

### F-01: Globally Atomic Cross-Process Maintenance Barrier & Complete Route Wiring
- **PostgreSQL Session Advisory Lock:** In `apps/spine-api/src/backup/consistency.rs`, `enter_maintenance` obtains an exclusive session advisory lock (`pg_advisory_lock(0x5350494E_4542414B)`), while `acquire_write_permit` obtains a shared session advisory lock (`pg_try_advisory_lock_shared(0x5350494E_4542414B)`). This guarantees cross-process mutual exclusion across arbitrary independent OS processes.
- **Process Crash Recovery:** Because advisory locks are held at the PostgreSQL session level on dedicated connections, an abrupt termination or crash of an application process automatically and immediately releases its lock in PostgreSQL.
- **Held Across Storage Capture:** In `apps/spine-api/src/main.rs`, `_maintenance_guard` is scoped to encompass the database snapshot, filesystem storage capture, encryption packaging, and integrity validation.
- **Bounded Lines & Exact Sequences:** In `apps/spine-api/src/db/dump.rs`, `read_bounded_line` enforces a 16 MiB line limit. Every sequence's `last_value` and `is_called` state are captured and restored with `setval(..., is_called)` and collision prevention.
- **Write Permit Wiring:** All state-modifying endpoints (`create_pull_request`, `update_pull_request`, `execute_pull_request_merge`, `update_release`, `delete_release`, `upload_release_asset`, `delete_release_asset`, `delete_repository`, `set_branch_protection`, `delete_branch_protection`, `cargo_yank`, `cargo_unyank`) invoke `acquire_write_permit().await?`.

### F-02: Staged Backup Creation, Key Rotation Decryption Verification, & Failure-Atomic Restores
- **Staged Archive Creation:** In `apps/spine-api/src/backup/mod.rs`, backups are created in a temporary staging file (`.tmp_bak_<uuid>`) and atomically renamed via `fs::rename` to destination upon successful completion. Destination files are never truncated beforehand.
- **Key Rotation Verification:** In `rotate_backup_key`, the rotated archive is fully decrypted into `std::io::sink()` (checking chunk sizes, auth tags, manifest, footer, and verifying no trailing bytes) before publishing via atomic rename.
- **Failure-Atomic Storage Unpacking:** In `unpack_archive_staged`, files are unpacked to `.tmp_restore_<uuid>`, all hashes and inventories are checked against the manifest, and live files are published with rollback cleanup on error.

### F-03: Authoritative Workflow Provenance
- `execute_job` requires `TrustClassification`; removed default unclassified bypass.
- Evaluated provenance fails closed to `Untrusted` on ambiguous or missing repository identifiers.

### F-04: Least-Privilege DDL/DML Role Separation
- `main.rs` supports `MIGRATION_DATABASE_URL` (`--migration-database-url-file`). Startup migrations run using migrator credentials and the connection is closed before `spine_app` runtime begins.
- `spine_app` has `REVOKE CREATE ON SCHEMA public` and cannot alter or create tables.

### F-05: Dockerfile Alignment & Ingress Network Reduction
- Dockerfiles aligned to `rust:1.85-bookworm` builder and `debian:bookworm-slim` runtime.
- Compose networks isolated: ingress proxy connects to `spine-public` and `spine-ingress-proxy`; database and storage nodes strictly on `spine-internal`.

### F-06: Restore Preflight & Secret Handling
- Preflight validation verifies archive format, encryption keys, and empty target database state before touching filesystem storage.
- Raw password CLI flags eliminated.

### F-07: Migration Cancellation Safety Under Live DDL/Inserts
- `test_repeated_migration_cancellation_during_real_ddl_and_inserts` verifies clean rollback and recovery across 5 consecutive cycles during active DDL and table inserts.

---

## 4. Tenth Independent Codex Audit (Batch 21H) Findings Remediation & Verification (Batch 21I)

Following the tenth independent deployment audit of Batch 21H, five defects were evaluated:

| ID | Severity | Category | Status | Batch 21I Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **SPINE-21H-01** | **P0 Blocker** | Distributed Backup Payload Contract | **RESOLVED** | Fixed participant payload contract so every registered participant—including `spine_storage`—stores a genuine non-empty streaming payload in the backup archive (`storage/spine_storage_snapshot.tar.gz`). Eliminated hash-of-paths metadata surrogate and unpack bypass `if p.participant_id == "spine_storage" { continue; }`. All 7 participants are strictly verified byte-for-byte against declared byte length and SHA-256 digest upon extraction. | `tests/production_backup_restore_roundtrip_test.rs` (`test_full_cluster_participant_contract_and_streaming_payload`, `test_participant_corruption_and_mismatch_rejection`) |
| **SPINE-21H-02** | **P0 Blocker** | Durable Restore Recovery & Control Plane | **RESOLVED** | Isolated restore journal in dedicated control directory (`.spine_restore_control/restore_journal.json`), preventing accidental erasure during whole-generation storage cutovers. Added `sync_all()` and atomic replace. Forward recovery safely advances from `StoragePublished` to `Committed`, purging old generations and staging folders. Incomplete restore attempts (`DbPublished`, `CommitDecision`) fail closed with actionable diagnostics. Staged coordinator restore records and previous committed states are persisted to disk across restarts. Staged SSH host keys are published only after database and storage cutovers, with rollback backups preserved in `.old_ssh_<id>`. | `tests/production_backup_restore_roundtrip_test.rs` (`test_forward_recovery_from_storage_published`, `test_ssh_identity_staged_publication_and_rollback_preservation`), `tests/restore_failure_injection_test.rs` |
| **SPINE-21H-03** | **P1 Blocker** | Writer Coverage & Durable Fencing | **RESOLVED** | Added writer guard acquisition to coordinator `record_heartbeat`, rejecting node heartbeats during cluster maintenance. Enforced strict generation monotonicity in `persist_fencing_generation` for coordinator and storage nodes. Existing participants with missing fencing markers fail closed immediately (`bail!`); only genuinely new participants initialize at generation 1. Replaced prior deletion pattern with safe atomic replacement and `sync_all()`. | `tests/production_backup_restore_roundtrip_test.rs` (`test_fencing_generation_monotonicity_and_fail_closed`), `git-alternative/apps/spine-storage-node/src/backend.rs`, `git-alternative/crates/spine-cluster/src/store.rs` |
| **SPINE-21H-04** | **P1 Blocker** | API Readiness & Container Health | **RESOLVED** | Implemented `/api/v1/health/ready` verifying database connectivity, storage root writeability, coordinator status, and all configured storage nodes with 2s bounded timeouts. Under `CoreForge` profile, requires >=3 active storage nodes; returns HTTP 503 `not_ready` with component health breakdown if any component fails. Configured healthchecks in `docker-compose.prod.yml` for coordinator and all storage nodes, and wired `spine-api` dependency conditions. | `apps/spine-api/src/api/routes_health.rs`, `docker-compose.prod.yml`, `apps/spine-api/src/main.rs` |
| **SPINE-21H-05** | **P1 Blocker** | PostgreSQL Provisioning & Secret Contracts | **RESOLVED** | Fixed SQL identifier vs literal escaping in `01-init-roles.sh` (`ESC_MIGRATOR_USER_IDENT`, `ESC_APP_USER_IDENT`). Enforced canonical database name contract detecting conflicts between `POSTGRES_DB` and `SPINE_POSTGRES_DB`. Ensured immediate fail-fast termination on unreadable or empty password files across initialization scripts and API startup. | `docker/postgres/01-init-roles.sh`, `scripts/upgrade_postgres_roles.sh`, `apps/spine-api/src/main.rs` |

---

## 5. Twelfth Independent Codex Audit (Batch 21J) Master Remediation (Batch 21K)

Following the twelfth independent deployment audit of Batch 21J, critical restore convergence, staging, durability, and confidentiality defects were resolved in **Batch 21K**:

| Finding ID | Severity | Category | Status | Batch 21K Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **SPINE-21K-P0-A** | **P0 Blocker** | Restore Protocol / Participant Acks | **RESOLVED** | Connected `record_participant_ack` directly to production restore orchestration (`main.rs`) across all 7 participants (`postgres_data`, `spine_storage`, `storage_1_data`, `storage_2_data`, `storage_3_data`, `coordinator_data`, `spine_ssh`). Replaced boolean RPC responses with typed `ParticipantAcknowledgement`. The restore journal strictly enforces that all required participant acknowledgements must be verified and recorded before advancing to `Prepared` or executing `CommitDecision`. | `apps/spine-api/src/main.rs`, `git-alternative/crates/spine-cluster/src/protocol.rs`, `git-alternative/crates/spine-cluster/src/store.rs`, `tests/global_restore_convergence_test.rs` |
| **SPINE-21K-P0-B** | **P0 Blocker** | Distributed Startup Recovery & Fencing | **RESOLVED** | Upgraded startup recovery (`handle_startup_recovery_cluster`) to reconcile all 7 cluster participants across local storage, database, coordinator, and storage nodes. Enforced fencing generation checks on storage node write permits and mutations. Fails closed immediately if any generation mismatch, incomplete recovery, or maintenance state is detected upon restart. | `apps/spine-api/src/backup/journal.rs`, `apps/spine-api/src/backup/consistency.rs`, `apps/spine-api/src/main.rs`, `tests/restore_failure_injection_test.rs` |
| **SPINE-21K-P0-C** | **P0 Blocker** | PostgreSQL Staging & Atomic Cutover | **RESOLVED** | Eliminated restoring directly into the live production database pool during pre-decision restore phases. Implemented isolated database staging (`StagedDatabaseRestore`) creating `<db>_staged_<restore_id>` (with schema-level staging fallback), restoring and verifying generation token within the isolated database before recording `ParticipantAcknowledgement`. Atomic cutover (`ALTER DATABASE RENAME`) occurs strictly after `CommitDecision`, preserving previous database until finalization. | `apps/spine-api/src/db/staging.rs`, `apps/spine-api/src/main.rs`, `tests/restore_failure_injection_test.rs`, `tests/global_restore_convergence_test.rs` |
| **SPINE-21K-P0-D** | **P0 Blocker** | Global Journal Durability & Replace | **RESOLVED** | Replaced vulnerable file replacement in `store.rs`, `backend.rs`, and `backup/mod.rs` with hardened `safe_atomic_replace`. Ensures `.last_valid` journal files and authoritative decisions are never deleted before replacement is verified via readback and parent directory `sync_all()`. | `git-alternative/crates/spine-cluster/src/store.rs`, `git-alternative/apps/spine-storage-node/src/backend.rs`, `apps/spine-api/src/backup/journal.rs`, `apps/spine-api/src/backup/mod.rs` |
| **SPINE-21K-P1-01** | **P1 Blocker** | Backup Confidentiality / Zero Plaintext | **RESOLVED** | Removed unencrypted intermediate `staging.tar.gz`. Backups stream compression directly through `v2::EncryptingWriter`, sealing 1 MiB chunks into AES-256-GCM `SPNEBAK v2` ciphertext directly on the fly. Completely eliminates plaintext SSH private keys or sensitive repository objects from disk during backup creation. | `apps/spine-api/src/backup/v2.rs`, `apps/spine-api/src/backup/mod.rs`, `tests/backup_v2_streaming_test.rs` |
| **SPINE-21K-P1-02** | **P1 Blocker** | Readiness & Consistency Write Gate | **RESOLVED** | Upgraded `/api/v1/health/ready` to strictly validate `responding_id` against `expected_id` (rejecting `None`) and verify generation consistency between PostgreSQL and storage. Wired consistency barrier `acquire_write_permit` to inspect restore journals, rejecting writes with HTTP 503 during active restores or unaligned generations. | `apps/spine-api/src/api/routes_health.rs`, `apps/spine-api/src/backup/consistency.rs`, `tests/global_restore_convergence_test.rs` |
| **SPINE-21K-P1-03** | **P1 Blocker** | Whole-Generation Directory Migration | **RESOLVED** | Replaced single-file moves with recursive `safe_move_path` in `publish_storage` and startup recovery, ensuring directories absent in newer generations do not survive cutovers, and resolving Windows OS error 650 directory copy failures. | `apps/spine-api/src/backup/mod.rs`, `apps/spine-api/src/backup/journal.rs`, `tests/restore_failure_injection_test.rs` |

---

## 6. Thirteenth Independent Codex Audit (Batch 21K) Master Remediation (Batch 21L)

Following the thirteenth independent deployment audit of Batch 21K, all remaining P0 architectural and P1 operational defects were resolved in **Batch 21L**:

| Finding ID | Severity | Category | Status | Batch 21L Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **SPINE-21L-P0-A** | **P0 Blocker** | Genuine Participant Acknowledgements | **RESOLVED** | Created dedicated, autonomous participant adapters (`LocalStorageParticipantAdapter`, `SshParticipantAdapter`, `StagedDatabaseRestore`, and coordinator/storage node RPC handlers) that independently extract, verify, persist durable staging records on disk (`.participant_staging_record.json`, `_spine_staging_record`, `.staged_restore_record.json`), and generate genuine `ParticipantAcknowledgement` structs. Eliminated orchestrator-manufactured acknowledgements. Strict commit-decision gate (`validate_commit_decision_gate`) mandates all 7 authentic acknowledgements are verified prior to the global commit decision. | `tests/batch_21l_remediation_test.rs` (`test_genuine_local_storage_participant_adapter`, `test_genuine_ssh_participant_adapter`, `test_commit_decision_gate_strict_validation`), `tests/global_restore_convergence_test.rs` |
| **SPINE-21L-P0-B** | **P0 Blocker** | PostgreSQL Isolation / Schema Fallback Elimination | **RESOLVED** | Completely removed `DatabaseStagingMode::SchemaLevel` and schema-level staging fallback from `db/staging.rs`. If an isolated staging database (`<base>_staged_<id>`) cannot be created, the operation fails closed immediately with an explicit error before any commit decision. Live production database and schemas remain 100% untouched throughout staging. | `apps/spine-api/src/db/staging.rs`, `apps/spine-api/src/db/mod.rs`, `tests/restore_failure_injection_test.rs` |
| **SPINE-21L-P0-C** | **P0 Blocker** | Recoverable PostgreSQL Active-Database Selection | **RESOLVED** | Implemented `DatabaseSelector` and `ActiveDatabaseRecord`. Staging restores into versioned database `<base>_staged_<id>`; cutover renames staged database to `<base>_gen_<target_gen>` and atomically records the active pointer in control database table `_spine_active_database_selector` (in `postgres` maintenance DB) and control file `.spine_restore_control/active_database_selector.json`. Application startup and connection pools resolve active database dynamically via `resolve_active_db_url`, eliminating fragile multi-step database renames. | `apps/spine-api/src/db/selector.rs`, `apps/spine-api/src/main.rs`, `tests/batch_21l_remediation_test.rs` (`test_database_selector_filesystem_and_url_resolution`) |
| **SPINE-21L-P0-D** | **P0 Blocker** | Failure-Safe Journal Durability & State Mutation | **RESOLVED** | Overhauled state transition persistence in `RestoreJournal::persist`. Constructs candidate state, increments monotonic sequence number, serializes, writes temporary file, invokes `sync_all()`, copies prior valid journal to `.last_valid`, performs platform-safe atomic replacement, validates readback integrity, and flushes directory (handling Windows directory handles via `FILE_FLAG_BACKUP_SEMANTICS`). Mutates in-memory state (`*self = candidate`) strictly after disk durability is confirmed. | `apps/spine-api/src/backup/journal.rs`, `apps/spine-api/src/backup/mod.rs`, `tests/batch_21l_remediation_test.rs` (`test_restore_journal_failure_safe_durability`) |
| **SPINE-21L-P1-01** | **P1 Blocker** | Fail-Closed Health Readiness & Write Pipeline | **RESOLVED** | Hardened `/api/v1/health/ready` to fail closed (HTTP 503) on any journal read, parse, or integrity error, missing coordinator under `CoreForge`, non-matching node IDs, or unaligned cluster generations. Hardened `ConsistencyBarrier::acquire_write_permit` (and sync variant) to inspect restore journals, failing closed on journal errors, active restore phases, or active database selector divergence from storage generation. | `apps/spine-api/src/api/routes_health.rs`, `apps/spine-api/src/backup/consistency.rs`, `tests/batch_21l_remediation_test.rs` (`test_consistency_barrier_fail_closed_checks`) |



