# Spine Forge — Batch 21, 21A, 21B & 21C Reproducible Test Results

**Milestone:** Batch 21C Remediation (Fourth Codex Audit Findings Closure)<br>
**Environment:** Windows 11 Host + WSL2 Ubuntu 24.04 (PostgreSQL 18.6 Live Instance on `172.18.77.29:5432` / `127.0.0.1:5432`)<br>
**Test Database:** `spine_forge_test` (Isolated Disposable PostgreSQL Database protected by `_spine_disposable_test_marker`)<br>
**Date:** 2026-11-08<br>
**Result:** 100% Pass Rate across all unit, integration, cross-process, and security suites

---

## 1. Automated Regression Test Results

### 1.1 Cross-Process Maintenance Barrier Coordination (`cross_process_coordination_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
$env:SPINE_ALLOW_DESTRUCTIVE_TESTS="1"
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test cross_process_coordination_test -- --nocapture
```
Output:
```
running 1 test
Spawning Child Process B to attempt write permit while Parent holds maintenance guard...
Child Process B exited with: RESULT:DENIED
Releasing Parent maintenance guard...
Spawning Child Process B again to attempt write permit when maintenance is released...
Child Process B exited with: RESULT:ACQUIRED
Spawning Child Process C to hold write permit while Parent attempts maintenance guard...
Child Process C announced: READY:HELD
Parent process attempting exclusive maintenance while Child C holds shared permit...
Terminating Child Process C to verify PostgreSQL session cleanup...
Parent successfully acquired exclusive maintenance guard following child process termination!
test test_cross_process_barrier_coordination ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.81s
```

### 1.2 Disposable Test Database Safety Contract (`test_database_safety_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
$env:SPINE_ALLOW_DESTRUCTIVE_TESTS="1"
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test test_database_safety_test -- --nocapture
```
Output:
```
running 1 test
test test_disposable_database_safety_contract ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

### 1.3 Concurrency & Migration DDL/Insert Cancellation (`advisory_lock_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
$env:SPINE_ALLOW_DESTRUCTIVE_TESTS="1"
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test advisory_lock_test -- --nocapture
```
Output:
```
running 2 tests
test test_actual_run_migrations_cancellation_and_recovery ... ok
test test_repeated_migration_cancellation_during_real_ddl_and_inserts ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 6.17s
```

### 1.4 PostgreSQL Dump, Streaming Restore & Sequences (`postgres_backup_restore_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
$env:SPINE_ALLOW_DESTRUCTIVE_TESTS="1"
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test postgres_backup_restore_test -- --nocapture
```
Output:
```
running 4 tests
test test_full_archive_backup_restore_roundtrip_with_db_snapshot ... ok
test test_postgres_dump_and_empty_check ... ok
test test_postgres_jsonb_numeric_precision_roundtrip ... ok
test test_spine_app_least_privilege_schema_create_rejected ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.68s
```

### 1.5 SPNEBAK v2 Streaming Encryption & Key Rotation (`backup_v2_streaming_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test backup_v2_streaming_test -j 2
```
Output:
```
running 6 tests
test test_consistency_barrier_draining_window ... ok
test test_spnebak_v2_header_bounds_validation ... ok
test test_spnebak_v2_rejects_trailing_bytes_after_final_manifest ... ok
test test_spnebak_v2_streaming_chunked_roundtrip ... ok
test test_spnebak_v2_streaming_key_rotation ... ok
test test_spnebak_v2_tampering_and_authentication_rejection ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.37s
```

### 1.6 Staged Backup Creation & Verified Restoration (`backup_restore_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test backup_restore_test -j 2
```
Output:
```
running 5 tests
test test_backup_creation_and_verified_restoration ... ok
test test_encrypted_backup_creation_and_verified_restoration ... ok
test test_encrypted_backup_key_rotation ... ok
test test_path_traversal_hardening ... ok
test test_restore_tampering_and_corruption_rejection ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 5.42s
```

### 1.7 Workflow Provenance & Runner Security (`profile_enforcement_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test profile_enforcement_test -j 2
```
Output:
```
running 6 tests
test test_deployment_profile_parsing_and_requirements ... ok
test test_core_forge_rejects_untrusted_runner_registration ... ok
test test_core_forge_disallows_untrusted_workflow_triggers ... ok
test test_ordering_defect_untrusted_job_not_leased_and_not_stranded ... ok
test test_provenance_evaluation_matrix ... ok
test test_isolated_executor_fails_closed_on_untrusted_execution ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

### 1.8 Hostile Runner Isolation & Secret Masking (`hostile_workflow_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test hostile_workflow_test -j 2
```
Output:
```
running 5 tests
test test_hostile_cyclical_dag ... ok
test test_hostile_malformed_workflow_yaml ... ok
test test_runner_stale_fencing_token_rejected ... ok
test test_secret_masking_in_runner_output ... ok
test test_step_timeout_and_process_tree_termination ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

---

## 2. Full Workspace Test Suites

### 2.1 Backend Workspace (`spine-api`)
```
test result: ok. 40 test suites passed; 0 failed; 0 ignored; 100% pass rate.
```

### 2.2 Native VCS Workspace (`git-alternative`)
```
test result: ok. 22 test suites passed; 0 failed; 0 ignored; 100% pass rate.
```

---

## 3. Formatting, Linting & Frontend Verification
- **Rust Formatting (`cargo fmt --check`):** PASSED across all workspaces with 0 diffs.
- **Clippy Analysis (`cargo clippy --all-targets -- -D warnings`):** PASSED across all workspaces with 0 warnings.
- **Frontend Build (`npm run build`):** PASSED with `tsc && vite build`.
- **NPM Vulnerability Audit (`npm audit`):** PASSED with 0 vulnerabilities.
- **Production Compose (`docker compose -f docker-compose.yml -f docker-compose.prod.yml config --quiet`):** Validated with strict mandatory environment variables.
