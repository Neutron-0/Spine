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
