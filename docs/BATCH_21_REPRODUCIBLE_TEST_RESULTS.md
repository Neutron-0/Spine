# Spine Forge — Batch 21, 21A & 21B Reproducible Test Results

**Milestone:** Batch 21B Remediation (Third Codex Audit Findings Closure)<br>
**Environment:** Windows 11 Host + WSL2 Ubuntu 24.04 (PostgreSQL 18.6 Live Instance on `172.18.77.29:5432`)<br>
**Test Database:** `spine_forge_test` (Isolated Disposable PostgreSQL Database)<br>
**Date:** 2026-11-07<br>
**Result:** 100% Pass Rate across all unit, integration, and security suites

---

## 1. Automated Regression Test Results

### 1.1 Concurrency & Advisory Lock Lifecycle (`advisory_lock_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test advisory_lock_test -- --nocapture
```
Output:
```
running 1 test
test test_actual_run_migrations_cancellation_and_recovery ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.22s
```

### 1.2 PostgreSQL Dump, Streaming Restore & Numeric Precision (`postgres_backup_restore_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_backup_restore_test -- --nocapture
```
Output:
```
running 4 tests
test test_spine_app_least_privilege_schema_create_rejected ... ok
test test_postgres_dump_and_empty_check ... ok
test test_postgres_jsonb_numeric_precision_roundtrip ... ok
test test_full_archive_backup_restore_roundtrip_with_db_snapshot ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.13s
```

### 1.3 SPNEBAK v2 Streaming Encryption & Key Rotation (`backup_v2_streaming_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test backup_v2_streaming_test -j 2
```
Output:
```
running 6 tests
test test_spnebak_v2_header_bounds_validation ... ok
test test_consistency_barrier_draining_window ... ok
test test_spnebak_v2_rejects_trailing_bytes_after_final_manifest ... ok
test test_spnebak_v2_streaming_chunked_roundtrip ... ok
test test_spnebak_v2_tampering_and_authentication_rejection ... ok
test test_spnebak_v2_streaming_key_rotation ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.39s
```

### 1.4 PostgreSQL Least-Privilege Role Hardening (`postgres_least_privilege_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_least_privilege_test -j 2
```
Output:
```
running 1 test
test test_spine_app_role_has_no_superuser_privileges ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.17s
```

### 1.5 Live PostgreSQL Identity, VCS & Persistence (`postgres_live_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_migrator:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_live_test -j 2
```
Output:
```
running 4 tests
test test_postgres_live_persistence_across_connections ... ok
test test_postgres_live_repository_and_permissions ... ok
test test_postgres_live_identity_and_auth_lifecycle ... ok
test test_postgres_live_migrations_and_concurrency ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.58s
```

### 1.6 Profile Enforcement & Runner Provenance Ordering (`profile_enforcement_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test profile_enforcement_test -j 2
```
Output:
```
running 6 tests
test test_deployment_profile_parsing_and_requirements ... ok
test test_core_forge_disallows_untrusted_workflow_triggers ... ok
test test_core_forge_rejects_untrusted_runner_registration ... ok
test test_provenance_evaluation_matrix ... ok
test test_ordering_defect_untrusted_job_not_leased_and_not_stranded ... ok
test test_isolated_executor_fails_closed_on_untrusted_execution ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 1.7 Failure Injection & Fencing (`failure_injection_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test failure_injection_test -j 2
```
Output:
```
running 3 tests
test test_fencing_token_rejects_stale_runner_lease_renewal ... ok
test test_unresponsive_runner_heartbeat_timeout_and_requeue ... ok
test test_step_failure_and_exit_code_propagation ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.04s
```

### 1.8 End-to-End Workflow CI & Protection (`workflow_ci_e2e_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test workflow_ci_e2e_test -j 2
```
Output:
```
running 1 test
test test_full_workflow_ci_dag_execution_and_pr_check_protection ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.88s
```

---

## 2. Code Quality & Linting Results

### 2.1 Workspace Formatting Check
```bash
cargo fmt --manifest-path github-alternative/backend/Cargo.toml --all -- --check
cargo fmt --manifest-path git-alternative/Cargo.toml --all -- --check
```
Output: `exit code 0` (clean, zero formatting differences).

### 2.2 Strict Clippy Lints
```bash
cargo clippy --manifest-path github-alternative/backend/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path git-alternative/Cargo.toml --all-targets -- -D warnings
```
Output: `exit code 0` (clean, zero warnings, zero errors).

### 2.3 Frontend Build & Audit
```bash
cd github-alternative/frontend && npm run build && npm audit
```
Output:
```
vite v6.4.5 building for production...
✓ 27 modules transformed.
dist/index.html                   0.50 kB
dist/assets/index-Cvc-eOIK.css    6.85 kB
dist/assets/index-BFubaDBS.js   276.47 kB
✓ built in 1.12s
found 0 vulnerabilities
```
