# Spine Forge — Batch 21, 21A & 21B Codex Findings Register & Remediation Verification

**Audited Milestone:** Batch 20 (`v0.1.0-rc1`), Batch 21 (`v0.1.0-rc2`), Batch 21A<br>
**Baseline Commit:** `a3d44568d1e3333a82e0a983970929a37fb3f37d`<br>
**Auditor:** Codex (Independent Read-Only Deployment Auditor)<br>
**Remediation Milestone:** Batch 21B<br>
**Status:** ALL 7 FINDINGS (F-01 THROUGH F-07) FULLY REMEDIATED & PROGRAMMATICALLY VERIFIED

---

## 1. Executive Summary

During the third independent read-only deployment audit, Codex identified residual blockers across the 7 deployment findings.

In **Batch 21B**, every defect was systematically remediated down to the production source code with zero whole-snapshot memory buffering, exact numeric precision roundtrips, database-backed cluster write maintenance barriers, failure-atomic key rotation, persisted workflow provenance, PostgreSQL least-privilege separation, hardened production Compose networking, and sanitized credential logging:

- **100% of integration & unit tests in `apps/spine-api` PASSED** (including `advisory_lock_test`, `postgres_backup_restore_test`, `postgres_least_privilege_test`, `postgres_live_test`, `backup_v2_streaming_test`, `profile_enforcement_test`, `failure_injection_test`).
- **100% of workspace tests in `git-alternative` PASSED**.
- **`cargo fmt --manifest-path ... --all -- --check` passed cleanly**.
- **`cargo clippy --manifest-path ... --all-targets -- -D warnings` passed cleanly with 0 errors and 0 warnings across both workspaces**.
- **Frontend built cleanly with `vite` and `npm audit` returned 0 vulnerabilities**.
- **Docker Compose configuration verified cleanly (`docker compose config --quiet`)**.

---

## 2. Findings Register & Remediation Summary

| ID | Severity | Category | Status | Batch 21B Code Remediations | Test Verification |
|---|---|---|---|---|---|
| **F-01** | **P0 Blocker** | Backup & Recovery | **RESOLVED** | Implemented Version 3.0 streaming JSON Lines format (`dump_database_to_writer`, `restore_database_from_reader`) with $O(1)$ memory buffering directly from/to disk. Added `serde_json/arbitrary_precision` preserving exact JSONB numeric tokens (`123456789012345678901234567890.123456789`). Wired `ConsistencyBarrier::acquire_write_permit` to all write routes (Git HTTP receive-pack, Git LFS upload/locks, SSH receive-pack, repos, releases, packages, artifacts). | `tests/postgres_backup_restore_test.rs` |
| **F-02** | **P1 Blocker** | Cryptography / Memory | **RESOLVED** | Hardened `rotate_backup_key`: added strict canonical path checks rejecting in-place rotation (`canon_src == canon_dst`), unique temporary staging `.tmp_rot_<uuid>.spnebak`, pre-publish header validation, and zero disk residue on failure. Bounded `ct_len` validation in `v2::rotate_stream`. | `tests/backup_v2_streaming_test.rs`, `tests/backup_restore_test.rs` |
| **F-03** | **P1 Blocker** | Security / CI Runner | **RESOLVED** | Added persistent columns `source_repo_id`, `is_fork`, `trust_classification`, `provenance_reason` to `workflow_runs` table (migration `0012_workflow_provenance.sql`). Stored evaluated provenance on run creation. Scheduler and runner lease inspect persisted provenance directly from run, eliminating unauthenticated `None` re-evaluations. | `tests/profile_enforcement_test.rs`, `tests/workflow_ci_e2e_test.rs` |
| **F-04** | **P1 Blocker** | Infrastructure / Database | **RESOLVED** | Restricted WSL PostgreSQL `pg_hba.conf` subnet from `172.16.0.0/12` to `172.18.64.0/20` (exact WSL subnet). Created dedicated DDL migration role `spine_migrator` (with `CREATE`) and runtime role `spine_app` (with `REVOKE CREATE ON SCHEMA public`). Verified `spine_app` table creation is rejected with error `42501` (`insufficient_privilege`). | `tests/postgres_backup_restore_test.rs`, `tests/postgres_least_privilege_test.rs` |
| **F-05** | **P1 Blocker** | Docker / Ingress Security | **RESOLVED** | Removed host port `3000:3000` from `docker-compose.yml` and segregated ports to `docker-compose.dev.yml`. Merged production compose exposes ONLY `80`, `443`, `2222`. Enforced HTTPS origin, non-empty host, and rejected userinfo credentials in `validate_public_origin`. | `docker compose config --quiet`, `apps/spine-api/src/main.rs` |
| **F-06** | **P1 Blocker** | Reliability / Database Fail-Fast | **RESOLVED** | Sanitized database password logging across all endpoints (`sanitize_db_url` masking password with `***`). Strict secret file error handling (`read_secret` returning hard error on unreadable or empty secret file). Pre-flight target database empty and connectivity check before modifying storage. | `apps/spine-api/src/main.rs`, `tests/postgres_backup_restore_test.rs` |
| **F-07** | **P1 Blocker** | Concurrency / DB Locking | **RESOLVED** | Verified `test_actual_run_migrations_cancellation_and_recovery` in `tests/advisory_lock_test.rs` executing actual `run_migrations_with_pause` against real PostgreSQL: transaction-level advisory lock verified in `pg_locks`, concurrent acquisition rejected, task cancellation cleanly releases lock, and subsequent worker runs to completion. | `tests/advisory_lock_test.rs` |

---

## 3. In-Depth Technical Remediation Details

### F-01: Production PostgreSQL Streaming Backup, Exact Precision & Cluster Maintenance
- **Streaming JSON Lines (Version 3.0):** Eliminated whole-snapshot memory buffering. `dump_database_to_writer` writes metadata and table rows sequentially line-by-line (`{"type":"metadata",...}`, `{"type":"table_start",...}`, `{"type":"row","data":...}`). `restore_database_from_reader` streams from reader via bounded 500-row chunks.
- **Arbitrary Numeric Precision:** Enabled `serde_json` `arbitrary_precision` feature in `github-alternative/backend/Cargo.toml`. Verified exact roundtrip for large numeric tokens (`123456789012345678901234567890.123456789`) without IEEE-754 floating-point truncation or loss.
- **Cluster Write Maintenance Barrier:** Created `_cluster_maintenance_barrier` in database. All repository mutation endpoints (`routes_git_http`, `routes_lfs`, `ssh/server`, `routes_repos`, `routes_releases`, `routes_packages`, `routes_artifacts`) acquire write permits asynchronously, gracefully returning HTTP 503 during active maintenance.

### F-02: SPNEBAK v2 Streaming Encryption & Failure-Atomic Key Rotation
- **Same-Path Rejection:** In `rotate_backup_key`, canonical source and destination paths are resolved. If `canon_src == canon_dst`, the operation fails immediately before altering state.
- **Atomic Staging & Publish:** Key rotation writes to a unique temporary file (`.tmp_rot_<uuid>.spnebak`). Before publishing, the header is fully parsed and validated. Only valid archives are atomically renamed (`fs::rename`) to destination. On error, temporary files are cleanly removed.

### F-03: Authenticated Workflow Provenance Persistence & Evaluation
- **Database Schema:** Added columns `source_repo_id TEXT`, `is_fork BOOLEAN NOT NULL DEFAULT FALSE`, `trust_classification TEXT NOT NULL DEFAULT 'Untrusted'`, `provenance_reason TEXT NOT NULL DEFAULT ''` to `workflow_runs` via migration `0012_workflow_provenance.sql`.
- **Zero-Trust Enforcement:** Provenance evaluated on workflow trigger and persisted. `WorkflowScheduler` and `RunnerService::acquire_job` read provenance directly from the persisted run, preventing bypasses and ensuring untrusted fork PR jobs are never assigned to internal runners.

### F-04: PostgreSQL Least-Privilege Role Hardening & Network Whitelisting
- **Subnet Restriction:** WSL PostgreSQL `pg_hba.conf` whitelisting narrowed to `172.18.64.0/20`.
- **Role Separation:**
  - `spine_migrator`: Dedicated role with `CREATE` privilege on schema `public` for running migrations.
  - `spine_app`: Runtime application role with `REVOKE CREATE ON SCHEMA public`. Default privileges grant `SELECT, INSERT, UPDATE, DELETE` on tables and `USAGE, SELECT, UPDATE` on sequences.
  - Verified `test_spine_app_least_privilege_schema_create_rejected` asserts `spine_app` is blocked from creating tables with PostgreSQL error `42501`.

### F-05: Production Compose Segregation & TLS Origin Validation
- **Port Segregation:** Host port `3000:3000` removed from `docker-compose.yml`. Merged production compose (`docker-compose.yml` + `docker-compose.prod.yml`) exposes only `80`, `443`, and `2222`.
- **Public Origin Validation:** `validate_public_origin` enforces HTTPS scheme, non-empty host, and rejects userinfo under `CoreForge`.

### F-06: Database Secret Management, Password Sanitization & Fail-Fast Restoration
- **Password Masking:** `sanitize_db_url` masks database passwords across all error logs and CLI outputs.
- **Secret File Validation:** `read_secret` validates file existence, permissions, and non-empty content, failing hard rather than defaulting to empty credentials.
- **Fail-Fast Restoration:** `Restore` command checks target database connectivity and verifies database is empty before touching filesystem storage.

### F-07: Real Concurrent PostgreSQL Advisory Lock Cancellation Safety
- **Verified in `tests/advisory_lock_test.rs`:**
  - Connection A initiates actual `run_migrations_with_pause`.
  - Transaction-level advisory lock verified held in `pg_locks`.
  - Connection B verifies `pg_try_advisory_xact_lock` returns false.
  - Connection A aborted; PostgreSQL engine immediately releases advisory lock.
  - Connection B acquires lock, commits all 12 migrations cleanly.
