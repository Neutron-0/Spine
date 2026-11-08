# SPINE — BATCH 21D REPRODUCIBLE TEST RESULTS
## Full Test Suite Execution, Cross-Process Evidence and Safety Proofs

**Date:** 2026-11-08  
**Target HEAD:** Batch 21D deployment readiness remediation  
**Operating System:** Windows 11 / WSL2 Linux (PostgreSQL 18 backend)  
**Rust Version:** rustc 1.88.0-nightly / stable toolchain compatible (MSRV 1.88)  

---

### 1. Disposable Database Safety Contract (`test_database_safety_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 31.12s)
- **Scenarios Verified:**
  1. Ephemeral database creation with unique name (`spine_disp_<uuid>`).
  2. Proof table recording (`_harness_provision_proof` with PID and timestamp).
  3. Strict refusal to operate on pre-existing databases (even if specified via URL).
  4. Automatic isolation from primary and template databases (`spine_forge`, `postgres`, `template1`).
  5. Destruction and cleanup of disposable database without affecting co-located databases.

---

### 2. Cross-Process Writer Coordination (`cross_process_coordination_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 5.86s)
- **Scenarios Verified:**
  1. **Writer Denied During Active Maintenance:** Subprocess acquiring exclusive session advisory lock (`0x5350494E_4542414B`) immediately causes parent writer permit acquisition to return `ForgeError::ServiceUnavailable("Cluster in maintenance")`.
  2. **Concurrent Maintenance Excluded:** Second process attempting `enter_maintenance` while another process holds exclusive maintenance lock times out and is rejected.
  3. **In-Flight Writer Drains Maintenance:** Active writer permit delays maintenance entry until writer completes and releases its permit.
  4. **Process Termination Recovery:** Killing an external process holding an exclusive advisory lock causes the PostgreSQL kernel to immediately drop all session locks, permitting subsequent permits and maintenance sessions to proceed cleanly.

---

### 3. F-07 Migration Advisory Locking (`advisory_lock_test.rs`)
- **Status:** PASS (2 passed; 0 failed; finished in 10.98s)
- **Scenarios Verified:**
  1. Transaction-level advisory lock mutual exclusion during schema migrations.
  2. Database-scoped lock accounting (`database = (SELECT oid FROM pg_database WHERE datname = current_database())`).
  3. Clean release and task cancellation without orphaned lock state.

---

### 4. Authoritative Workflow Provenance (`hostile_workflow_test.rs`, `profile_enforcement_test.rs`, `workflow_ci_e2e_test.rs`)
- **Status:** PASS (12 passed; 0 failed; finished in 2.07s)
- **Scenarios Verified:**
  1. `IsolatedRunnerExecutor::execute_job` mandates `AuthoritativeTrustToken` derived from database lookup.
  2. Untrusted workflow execution strictly rejected under `core-forge-v1` deployment profile.
  3. Secret masking verified in runner output (ciphertext replaced with `***`).
  4. Runner stale fencing token rejected; job timeout cancels gracefully.
  5. End-to-end DAG execution across multi-step pipeline with PR check protection.

---

### 5. PostgreSQL Backup & Restore with High-Precision JSONB (`postgres_backup_restore_test.rs`)
- **Status:** PASS (4 passed; 0 failed; finished in 2.25s)
- **Scenarios Verified:**
  1. Database snapshot streaming dump and restore (JSON Lines format 3.0).
  2. Non-empty target database rejection (fail-fast conflict check).
  3. Exact 30-digit integer + 9-digit fractional JSONB numeric precision preserved character-for-character without floating-point conversion.
  4. Full archive backup and restore roundtrip with database snapshot and repository storage.

---

### 6. Durable Cross-Resource Restore & Crash Recovery (`restore_failure_injection_test.rs`)
- **Status:** PASS (3 passed; 0 failed; finished in 1.07s)
- **Scenarios Verified:**
  1. **Database Restore Failure Leaves Live Storage Untouched:** Live repository files (`config.toml`, `spine.db`) remain 100% identical when a database restore error is simulated during restore.
  2. **Successful Restore Atomically Publishes Storage:** Staged files are moved to live storage root only after database restore succeeds.
  3. **Startup Crash Recovery:** Orphaned staging directory and interrupted journal in `RestorePhase::StagedStorage` are cleanly rolled back on subsequent server startup.

---

### 7. PostgreSQL Least Privilege (`postgres_least_privilege_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 1.35s)
- **Scenarios Verified:**
  1. `spine_app` rolsuper, rolcreaterole, rolcreatedb, rolbypassrls are all `false`.
  2. `spine_app` attempting `CREATE ROLE` is rejected with permission denied.
  3. `spine_app` attempting `CREATE TABLE` on schema `public` is rejected (SQLSTATE 42501).
  4. `spine_app` possesses valid DML (`SELECT count(*) FROM users`) on application tables.

---

### 8. Container Toolchain & Formatting Verification
- `cargo fmt --all -- --check`: PASS (0 differences)
- `cargo clippy --all-targets -j 2 -- -D warnings` (git-alternative): PASS (0 warnings)
- `cargo clippy --all-targets -j 2 -- -D warnings` (github-alternative/backend): PASS (0 warnings)
- Docker execution note: Docker engine pipe is unmounted on host; Compose deployment verification status reported honestly as `NOT VERIFIED` on daemon. All configurations, entrypoints, and role scripts are verified.
