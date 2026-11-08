# SPINE — BATCH 21E REPRODUCIBLE TEST RESULTS
## Full Test Suite Execution, Cross-Process Evidence and Safety Proofs

**Date:** 2026-11-08  
**Target HEAD:** Batch 21E deployment readiness closure  
**Operating System:** Windows 11 / PostgreSQL 16  
**Rust Version:** rustc 1.88.0-nightly / stable toolchain compatible (MSRV 1.88)  

---

### 1. Crash-Recoverable Restore State Machine & Fail-Closed Startup (`restore_failure_injection_test.rs`)
- **Status:** PASS (4 passed; 0 failed; finished in 1.54s)
- **Scenarios Verified:**
  1. `test_startup_crash_recovery_cleans_interrupted_staging`: Pre-publication crash during `StagedStorage` cleanly removes temporary directory and deletes journal without touching live storage.
  2. `test_startup_recovery_fails_closed_on_interrupted_db_cutover`: Interruption during `DbPublished` phase (database updated, storage not yet published) triggers fatal `handle_startup_recovery` failure, blocking server startup and preventing split-brain corruption.
  3. `test_restore_database_failure_leaves_live_storage_untouched`: Database restore failure during staging rolls back cleanly, leaving existing live repository files completely untouched.
  4. `test_restore_database_success_publishes_storage_atomically`: Successful restore sequences atomically transition `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`, publishing files, writing `.spine_storage_generation`, and recording database restore generation.

---

### 2. Disposable Database Safety Contract (`test_database_safety_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 0.00s)
- **Scenarios Verified:**
  1. Opt-in flag enforcement: Refuses destructive modifications without `SPINE_ALLOW_DESTRUCTIVE_TESTS=1`.
  2. Pre-existing database protection: Rejects pre-existing database targets missing current harness proof.
  3. Fail-closed provisioning: Network or unreachable database configuration fails immediately without performing SQL operations.

---

### 3. PostgreSQL Least-Privilege Role Isolation (`postgres_least_privilege_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 21.07s)
- **Scenarios Verified:**
  1. No silent fallback in harness: `app_pool` connects strictly as `spine_app`.
  2. `spine_app` privileges verified: `rolsuper=false`, `rolcreaterole=false`, `rolcreatedb=false`, `rolbypassrls=false`.
  3. Unauthorized administrative actions denied: `CREATE ROLE` and `CREATE TABLE on public schema` rejected with permission denied.
  4. Application DML queries executed successfully: `SELECT count(*) FROM users` succeeded.

---

### 4. Authoritative Workflow Provenance (`hostile_workflow_test.rs`, `profile_enforcement_test.rs`, `failure_injection_test.rs`)
- **Status:** PASS (16 passed; 0 failed)
- **Scenarios Verified:**
  1. `AuthoritativeTrustToken` constructor `for_test` restricted to integration test scopes (`#[cfg(any(test, feature = "test-support"))]`).
  2. `AuthoritativeTrustToken` strictly binds `run_id`, `job_id`, `repository_id`, and fork classification.
  3. Untrusted workflow execution strictly rejected under `core-forge-v1` deployment profile.
  4. Runner routes (`register_runner`, `heartbeat`, `acquire_job`, `ingest_logs`, `complete_job`) participate in global consistency barrier (`acquire_write_permit` and `verify_active`).

---

### 5. SPNEBAK v2 Streaming Encryption & Key Rotation (`backup_v2_streaming_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 2.36s)
- **Scenarios Verified:**
  1. Header format and magic byte bounds validation.
  2. Streaming chunked encryption and decryption roundtrip.
  3. Tampering and authentication tag verification rejection.
  4. Trailing byte injection rejection.
  5. In-place master key rotation without intermediate plaintext exposure.
  6. Maintenance barrier draining window coordination.

---

### 6. Workspace Static Analysis & Code Quality
- **`cargo fmt --check`:** PASSED (All crates formatted according to Rust style guidelines).
- **`cargo clippy --all-targets -- -D warnings`:** PASSED (Zero warnings across both workspaces).
- **Release Compilation:** All 4 release binaries compiled successfully with optimized release profiles.
