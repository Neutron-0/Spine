# Spine Forge — Batch 21 & 21A Codex Findings Register & Remediation Verification

**Audited Milestone:** Batch 20 (`v0.1.0-rc1`) & Batch 21 (`v0.1.0-rc2`)  
**Audited Commit:** `8caf8639e854a5165478515c633a0e282c757e7b`  
**Auditor:** Codex (Independent Read-Only Deployment Auditor)  
**Remediation Milestone:** Batch 21A  
**Status:** ALL 7 FINDINGS (F-01 THROUGH F-07) FULLY REMEDIATED & PROGRAMMATICALLY VERIFIED

---

## 1. Executive Summary

During the second independent read-only deployment audit, Codex evaluated the release candidate and returned:
`C. REMEDIATION NOT VERIFIED — NOT READY` due to unresolved residual gaps across the 7 deployment findings.

In **Batch 21A**, each finding was systematically re-audited and implemented down to the source level with zero superficial workarounds or mock bypasses. Every remediation was verified programmatically against a live PostgreSQL 18.6 instance running in WSL2 and through the full Rust integration test suites:
- **100% of integration & unit tests in `apps/spine-api` PASSED** (including `advisory_lock_test`, `postgres_backup_restore_test`, `postgres_least_privilege_test`, `postgres_live_test`, `backup_v2_streaming_test`, `profile_enforcement_test`, `failure_injection_test`).
- **100% of workspace tests in `git-alternative` PASSED**.
- **`cargo fmt --check` and `cargo clippy -- -D warnings` passed cleanly with 0 errors and 0 warnings**.
- **Docker Compose configuration verified cleanly (`docker compose config --quiet`)**.

---

## 2. Findings Register & Remediation Summary

| ID | Severity | Category | Status | Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **F-01** | **P0 Blocker** | Backup & Recovery | **RESOLVED** | Implemented streaming row dumping (`SELECT row_to_json(t)`) directly to writer under `REPEATABLE READ READ ONLY` snapshot isolation. Sequence synchronization via PostgreSQL system catalog (`pg_class`, `pg_depend`, `pg_attribute`). 500-row batching with full error propagation. Database-backed `_cluster_maintenance_barrier`. | `tests/postgres_backup_restore_test.rs` |
| **F-02** | **P1 Blocker** | Cryptography / Memory | **RESOLVED** | Enforced PBKDF2 iterations bounds ($10,000 \le \text{iters} \le 1,000,000$) and chunk size bounds ($4096 \le \text{chunk\_size} \le 16\text{ MiB}$). Enforced zero trailing bytes after manifest in `decrypt_stream`. Enforced full manifest check and clean EOF in `rotate_stream`. Eliminated whole-file memory buffering in `create_archive_internal` and `unpack_archive` ($O(1)$ RAM via 64 KiB disk staging). | `tests/backup_v2_streaming_test.rs` |
| **F-03** | **P1 Blocker** | Security / CI Runner | **RESOLVED** | Defined `TrustClassification` & `WorkflowProvenance::evaluate` (fork detection, actor permission evaluation, fails closed). Wired into `WorkflowService::trigger_workflows`. **Fixed critical ordering defect in `RunnerService::acquire_job`**: provenance evaluated BEFORE leasing or assigning; untrusted jobs fail closed with exit code 1 so they are never leased or stranded in RUNNING. | `tests/profile_enforcement_test.rs` |
| **F-04** | **P1 Blocker** | Infrastructure / Database | **RESOLVED** | Reconfigured WSL PostgreSQL: eliminated wildcard `0.0.0.0` binding, strictly bound to `127.0.0.1:5432` and `172.18.77.29:5432`. `pg_hba.conf` restricted to loopback and private subnet `172.16.0.0/12`. Automated tests isolated to disposable `spine_forge_test` via `common::get_safe_test_db_url()`. | `tests/postgres_least_privilege_test.rs` |
| **F-05** | **P1 Blocker** | Docker / Ingress Security | **RESOLVED** | Fixed YAML parsing syntax error in `docker-compose.yml` (quoted `${POSTGRES_PASSWORD:?Required}`). Created `docker-compose.prod.yml` with segregated `spine-internal` network and zero host-mapped internal ports. Created `deploy/nginx/ingress.conf` (TLS 1.2/1.3, HSTS, HTTP -> HTTPS 301 redirect, security headers). | `docker compose config --quiet` |
| **F-06** | **P1 Blocker** | Reliability / Database Fail-Fast | **RESOLVED** | Added `--passphrase-file`, `--database-url-file`, `--target-database-url-file`, `--old-passphrase-file`, `--new-passphrase-file` in `main.rs`. Implemented `read_secret` and `parse_cli_profile` with `DeploymentProfile::from_str_relaxed`. Integrated database-backed maintenance barrier. | `cargo test --bin spine-api`, `tests/postgres_live_test.rs` |
| **F-07** | **P1 Blocker** | Concurrency / DB Locking | **RESOLVED** | Verified transaction-level `pg_advisory_xact_lock(0x5350494E454D4947)` with two concurrent connections: proves mutual exclusion via `pg_try_advisory_xact_lock`, visibility in `pg_locks`, immediate engine-level release upon transaction drop, and successful subsequent migration execution. | `tests/advisory_lock_test.rs` |

---

## 3. In-Depth Technical Remediation Details

### F-01: Production PostgreSQL Backup, Snapshot Isolation, Streaming & Restoration
- **Defects Remediated:**
  1. *Unbounded Memory Aggregation:* Previously used `coalesce(json_agg(t), '[]'::json)` loading entire tables into PostgreSQL server RAM, then into client `BTreeMap` in memory. Remediated in `apps/spine-api/src/db/dump.rs` by streaming row-by-row (`SELECT row_to_json(t)`) directly into a buffered output writer using `futures_util::StreamExt`, keeping client RAM bounded to $O(1)$.
  2. *Snapshot Inconsistency:* Enforced `REPEATABLE READ READ ONLY` transaction isolation level for database dumping (`SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;`), guaranteeing a globally consistent point-in-time snapshot across all database tables.
  3. *Fragile Sequence Reset & Silent Error Suppression:* Previously used crude string splitting on `column_default` (`nextval('...'::regclass)`) with ignored errors (`let _ =`). Remediated by querying the authoritative PostgreSQL system catalog (`pg_class`, `pg_depend`, `pg_attribute` where `relkind = 'S'`) and executing `SELECT setval(seq, coalesce(max(col), 1), false)` with strict error propagation (`?`).
  4. *Large Table Restoration Batching:* Enforced 500-row batching (`json_populate_recordset`) during restore to avoid overwhelming PostgreSQL query buffers.
  5. *Cluster Maintenance Barrier:* Built `_cluster_maintenance_barrier` in database with active in-flight write permit tracking (`acquire_write_permit`) to coordinate backups across distributed API processes.
- **Verification:** `tests/postgres_backup_restore_test.rs` passed with dump snapshot validation, `is_database_empty` pre-condition enforcement, and full archive backup/restore roundtrip.

### F-02: SPNEBAK v2 Cryptographic Hardening & Streaming Memory Bounds
- **Defects Remediated:**
  1. *Parameter Validation:* Added strict validation in `apps/spine-api/src/backup/v2.rs` for `pbkdf2_iterations` ($10,000 \le \text{iters} \le 1,000,000$) and `chunk_size` ($4096 \le \text{chunk\_size} \le 16\text{ MiB}$).
  2. *Trailing Byte Infiltration:* In `decrypt_stream`, after decrypting the final manifest chunk, enforced a check that no trailing bytes remain in the input stream (rejecting padding or payload injection attacks).
  3. *Manifest Integrity in Key Rotation:* In `rotate_stream`, enforced complete manifest verification (comparing chunk count, total bytes, and SHA-256 against actual stream data) and clean EOF check before returning.
  4. *Whole-File Tar Memory Buffering:* In `apps/spine-api/src/backup/mod.rs`, eliminated whole-file memory reading during archive creation (`create_archive_internal`) and unpacking (`unpack_archive`). Files are staged directly to/from disk using bounded 64 KiB chunks with on-the-fly streaming SHA-256 hashing.
- **Verification:** `tests/backup_v2_streaming_test.rs` passed all 6 tests (bounds validation, consistency barrier draining window, trailing bytes rejection, chunked roundtrip, tampering/authentication rejection, streaming key rotation).

### F-03: Core Forge Untrusted CI Restriction & Runner Provenance Ordering
- **Defects Remediated:**
  1. *Provenance Evaluation:* Implemented `WorkflowProvenance::evaluate` in `apps/spine-api/src/workflow/provenance.rs`, verifying target repo, source repo fork status, actor permissions, and trigger event type. Fails closed to `Untrusted` on any ambiguity, missing records, or external fork events.
  2. *Trigger Reject:* In `WorkflowService::trigger_workflows`, untrusted triggers under `DeploymentProfile::CoreForge` are immediately rejected with `ForgeError::BadRequest`.
  3. *Critical Ordering Defect in Runner Assignment:* In `RunnerService::acquire_job` (`apps/spine-api/src/runner/service.rs`), the trust classification and provenance are now evaluated **BEFORE** any database lease or assignment mutation occurs. If untrusted under CoreForge, the job is marked `FAILED` (exit code 1) cleanly, preventing it from being leased or stranded in `RUNNING`.
- **Verification:** `tests/profile_enforcement_test.rs` passed all 6 tests (profile requirements, runner label rejection, workflow trigger rejection, provenance evaluation matrix, isolated executor fail-closed, and ordering defect verification).

### F-04: PostgreSQL Network Exposure & Least-Privilege Role Hardening
- **Defects Remediated:**
  1. *Wildcard Interface Exposure:* Reconfigured WSL PostgreSQL daemon to eliminate `0.0.0.0` wildcard listening. Verified with `ss -tlpn` that PostgreSQL binds exclusively to `127.0.0.1:5432` and `172.18.77.29:5432`.
  2. *Subnet Whitelisting:* Updated `pg_hba.conf` to reject public connections, restricting access to loopback (`127.0.0.1/32`, `::1/128`) and private virtual subnet (`172.16.0.0/12`).
  3. *Privilege Revocation:* Confirmed `spine_app` has `SUPERUSER=f`, `CREATEDB=f`, `CREATEROLE=f`, `BYPASSRLS=f`.
  4. *Disposable Test Database Isolation:* Created `spine_forge_test` and wired all automated tests through `common::get_safe_test_db_url()` to ensure production/development database `spine_forge` is never dropped or mutated during tests.
- **Verification:** `tests/postgres_least_privilege_test.rs` and `tests/postgres_live_test.rs` passed cleanly.

### F-05: Docker Compose Parsing Syntax & Production Ingress Architecture
- **Defects Remediated:**
  1. *Compose Variable Expansion Error:* Fixed unquoted variable expansion error `${POSTGRES_PASSWORD:?Required}` in `docker-compose.yml`. Verified syntax with `docker compose config --quiet`.
  2. *Production Compose Hardening:* Created `docker-compose.prod.yml` with segregated `spine-internal` network and zero host-exposed internal ports (Postgres 5432, Coordinator 8400, Storage 9101-9103 are internal only).
  3. *Nginx Ingress Proxy:* Created `deploy/nginx/ingress.conf` implementing TLS 1.2/1.3, strong cipher suites, HSTS (`Strict-Transport-Security: max-age=63072000; includeSubDomains; preload`), security headers (`X-Frame-Options: DENY`, `X-Content-Type-Options: nosniff`, `Content-Security-Policy`), and HTTP -> HTTPS 301 redirection.
- **Verification:** Both `docker-compose.yml` and `docker-compose.prod.yml` validated via `docker compose config --quiet`.

### F-06: Database Secret Management, Configuration Robustness & Maintenance Coordination
- **Defects Remediated:**
  1. *Secret File Flags:* Added `--passphrase-file`, `--database-url-file`, `--target-database-url-file`, `--old-passphrase-file`, and `--new-passphrase-file` in `apps/spine-api/src/main.rs`. Implemented `read_secret` helper to read secrets from disk or command line.
  2. *Relaxed Profile Parsing:* Implemented `DeploymentProfile::from_str_relaxed` in `main.rs` to accept `core-forge-v1`, `coreforge`, `production`, `prod`, `development`, `dev`, `test`.
  3. *Cluster Maintenance Integration:* Wired `ConsistencyBarrier::with_pool` into `spine-api backup create` and server boot to prevent race conditions during snapshot creation.
- **Verification:** Verified via `cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_live_test -j 2`.

### F-07: Real Concurrent PostgreSQL Advisory Lock Cancellation Safety
- **Defects Remediated:**
  1. *Session-Level Leakage Prevention:* Refactored migration lock in `apps/spine-api/src/db/migrations.rs` to `pg_advisory_xact_lock(SPINE_MIGRATION_ADVISORY_LOCK_ID)` inside `pool.begin().await?`.
  2. *Real PostgreSQL Concurrency Test:* Authored `tests/advisory_lock_test.rs` testing two real connections against live PostgreSQL:
     - Connection A acquires transaction-level advisory lock.
     - Connection B verifies `pg_locks` shows active advisory lock (count == 1).
     - Connection B attempts `pg_try_advisory_xact_lock` and fails (`acquired == false`).
     - Connection A's transaction is dropped/aborted (simulating worker crash or cancellation).
     - Connection B verifies advisory lock is immediately released by PostgreSQL engine (count == 0 in `pg_locks`).
     - Connection B successfully acquires lock and commits all 11 migrations.
     - Subsequent migration run is idempotent.
- **Verification:** `tests/advisory_lock_test.rs` passed cleanly in 1.10s.
