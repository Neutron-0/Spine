# Spine Forge — Batch 21A Reproducible Test Results

**Milestone:** Batch 21A Remediation (Second Codex Audit Findings Closure)  
**Environment:** Windows 11 Host + WSL2 Ubuntu 24.04 (PostgreSQL 18.6 Live Instance on `172.18.77.29:5432`)  
**Test Database:** `spine_forge_test` (Isolated Disposable PostgreSQL Database)  
**Date:** 2026-11-06  
**Result:** 100% Pass Rate across all unit, integration, and security suites

---

## 1. Automated Regression Test Results

### 1.1 Concurrency & Advisory Lock Lifecycle (`advisory_lock_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test advisory_lock_test -j 2
```
Output:
```
running 1 test
test test_actual_run_migrations_cancellation_and_recovery ... ok

test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.10s
```

### 1.2 SPNEBAK v2 Streaming Encryption & Key Rotation (`backup_v2_streaming_test.rs`)
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

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.35s
```

### 1.3 PostgreSQL Dump, Restore & Topological Sequence Resync (`postgres_backup_restore_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_backup_restore_test -j 2
```
Output:
```
running 2 tests
test test_postgres_dump_and_empty_check ... ok
test test_full_archive_backup_restore_roundtrip_with_db_snapshot ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.13s
```

### 1.4 PostgreSQL Least-Privilege Role Hardening (`postgres_least_privilege_test.rs`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
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
$env:SPINE_TEST_DATABASE_URL="postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_live_test -j 2
```
Output:
```
running 4 tests
test test_postgres_live_persistence_across_connections ... ok
test test_postgres_live_repository_and_permissions ... ok
test test_postgres_live_identity_and_auth_lifecycle ... ok
test test_postgres_live_migrations_and_concurrency ... ok

test result: ok. 4 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.54s
```

### 1.6 Profile Enforcement & Runner Provenance Ordering (`profile_enforcement_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test profile_enforcement_test -j 2
```
Output:
```
running 6 tests
test test_deployment_profile_parsing_and_requirements ... ok
test test_core_forge_rejects_untrusted_runner_registration ... ok
test test_core_forge_disallows_untrusted_workflow_triggers ... ok
test test_provenance_evaluation_matrix ... ok
test test_isolated_executor_fails_closed_on_untrusted_execution ... ok
test test_ordering_defect_untrusted_job_not_leased_and_not_stranded ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.00s
```

### 1.7 Security Failure Injection & Isolation (`failure_injection_test.rs`)
```bash
cargo test --manifest-path github-alternative/backend/Cargo.toml --test failure_injection_test -j 2
```
Output:
```
running 5 tests
test test_secret_fingerprinting_keyed_hmac_and_no_plaintext_leakage ... ok
test test_ssrf_validator_blocks_internal_addresses_and_prohibited_ports ... ok
test test_stale_fencing_token_and_runner_lease_invalidation ... ok
test test_observability_liveness_readiness_and_diagnostics_discrimination ... ok
test test_runner_isolation_fail_closed_on_unsandboxed_untrusted_workflow ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

---

## 2. Full Workspace Test Suites

### 2.1 Complete Backend Suite (`github-alternative/backend`)
```bash
$env:SPINE_TEST_DATABASE_URL="postgres://spine_app:spine_secure_dev_password@172.18.77.29:5432/spine_forge_test?sslmode=disable"
cargo test --manifest-path github-alternative/backend/Cargo.toml -j 2
```
Result: **100% of integration & unit tests passed (finished in 2m 51s, 0 failures)**.

### 2.2 Complete Spine VCS Workspace Suite (`git-alternative`)
```bash
cargo test --manifest-path git-alternative/Cargo.toml -j 2
```
Result: **100% of unit, integration, benchmark, and doc tests passed (0 failures)**.

---

## 3. Linter, Formatting & Static Configuration Validation

```bash
$ cargo fmt --manifest-path github-alternative/backend/Cargo.toml --all -- --check
Passed (0 diffs)

$ cargo clippy --manifest-path github-alternative/backend/Cargo.toml --all-targets -j 2 -- -D warnings
Finished `dev` profile [unoptimized + debuginfo] target(s) in 50.30s (0 errors, 0 warnings)

$ docker compose config --quiet
Valid (0 errors)

$ docker compose -f docker-compose.prod.yml config --quiet
Valid (0 errors)
```
