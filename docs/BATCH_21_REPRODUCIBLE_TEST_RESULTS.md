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

---

## 4. Batch 21K Master Remediation Test Evidence

**Milestone:** Batch 21K — Production Global Restore Recovery, PostgreSQL Staging, Journal Durability, and Backup Confidentiality<br>
**Baseline Commit:** `fa7bd142d4f42d8d86bb04d3c713067b944c77a9`<br>
**Environment:** Windows 11 Native Host (`x86_64-pc-windows-msvc`)<br>
**Execution Date:** 2026-11-10<br>
**Pass Rate:** 100% across all suites (zero failures, zero warnings).

### 4.1 SPNEBAK v2 Streaming AEAD Encryption (`backup_v2_streaming_test.rs`)
```text
running 6 tests
test test_spnebak_v2_header_bounds_validation ... ok
test test_consistency_barrier_draining_window ... ok
test test_spnebak_v2_rejects_trailing_bytes_after_final_manifest ... ok
test test_spnebak_v2_streaming_chunked_roundtrip ... ok
test test_spnebak_v2_tampering_and_authentication_rejection ... ok
test test_spnebak_v2_streaming_key_rotation ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.32s
```

### 4.2 Global Restore Convergence & Readiness Gate (`global_restore_convergence_test.rs`)
```text
running 6 tests
test test_canonical_seven_participants_contract_validation ... ok
test test_api_readiness_blocks_during_restore_and_clears_on_convergence ... ok
test test_crash_matrix_point_14_and_15_recovery_idempotence ... ok
test test_crash_matrix_point_5_to_9_post_decision_rolls_forward ... ok
test test_crash_matrix_point_4_decision_write_fallback_to_last_valid ... ok
test test_crash_matrix_point_1_to_3_pre_decision_aborts_cleanly ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.07s
```

### 4.3 Restore Failure Injection & Cutover Safety (`restore_failure_injection_test.rs`)
```text
running 6 tests
test test_startup_crash_recovery_cleans_interrupted_staging ... ok
test test_startup_recovery_rolls_forward_on_interrupted_db_cutover ... ok
test test_whole_generation_cutover_absent_files_do_not_survive ... ok
test test_multi_volume_cluster_backup_manifest_and_cutover_isolation ... ok
test test_restore_database_success_publishes_storage_atomically ... ok
test test_restore_database_failure_leaves_live_storage_untouched ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.56s
```

### 4.4 Distributed Participant Integrity (`distributed_participant_integrity_test.rs`)
```text
running 3 tests
test test_participant_registry_production_enforcement ... ok
test test_ssh_host_identity_capture_and_restore_roundtrip ... ok
test test_distributed_cutover_multi_phase_rollback_preservation ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.42s
```

### 4.5 Production Backup/Restore Roundtrip (`production_backup_restore_roundtrip_test.rs`)
```text
running 5 tests
test test_fencing_generation_monotonicity_and_fail_closed ... ok
test test_ssh_identity_staged_publication_and_rollback_preservation ... ok
test test_forward_recovery_from_storage_published ... ok
test test_participant_corruption_and_mismatch_rejection ... ok
test test_full_cluster_participant_contract_and_streaming_payload ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.18s
```

### 4.6 Batch 21L Master Remediation Test Suite (`batch_21l_remediation_test.rs`)
```text
running 6 tests
test test_genuine_ssh_participant_adapter ... ok
test test_genuine_local_storage_participant_adapter ... ok
test test_consistency_barrier_fail_closed_checks ... ok
test test_restore_journal_failure_safe_durability ... ok
test test_commit_decision_gate_strict_validation ... ok
test test_database_selector_filesystem_and_url_resolution ... ok

test result: ok. 6 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 4.33s
```

### 4.7 Complete Backend Test Suite (`github-alternative/backend`)
```text
42 integration and unit test binaries executed.
test result: ok. 100% passed; 0 failed; 0 ignored; 0 warnings.
```

### 4.8 Complete Native Cluster Test Suite (`git-alternative`)
```text
22 integration, unit, and doc test suites executed.
test result: ok. 100% passed; 0 failed; 0 ignored; 0 warnings.
```

### 4.9 Quality, Linting & Release Assets
- **Cargo Format:** `cargo fmt --all -- --check` exited with code 0 on both workspaces (`github-alternative/backend` and `git-alternative`).
- **Cargo Clippy:** `cargo clippy --all-targets -- -D warnings` exited with code 0 on both workspaces.
- **Release Binaries:** `spine-api.exe`, `spine.exe`, `spine-coordinator.exe`, `spine-storage-node.exe` rebuilt in release mode and checksummed in `target/release-candidate/SHA256SUMS`.
