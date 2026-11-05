# Spine Forge — Batch 21 Reproducible Test Results

**Milestone:** Batch 21 Remediation  
**Environment:** Windows 11 Host + WSL2 Ubuntu 24.04 (PostgreSQL 18.6 Live Instance)  
**Date:** 2026-11-05  
**Result:** 100% Pass Rate across all unit, integration, and security suites

---

## 1. Regression Test Summary

```
Running tests\advisory_lock_test.rs
test test_migration_advisory_xact_lock_lifecycle_and_cancellation_safety ... ok (0.35s)
Result: 1 passed; 0 failed

Running tests\backup_v2_streaming_test.rs
test test_consistency_barrier_draining_window ... ok
test test_spnebak_v2_streaming_chunked_roundtrip ... ok (2.5 MiB multi-chunk)
test test_spnebak_v2_tampering_and_authentication_rejection ... ok
test test_spnebak_v2_streaming_key_rotation ... ok
Result: 4 passed; 0 failed

Running tests\postgres_backup_restore_test.rs
test test_postgres_dump_and_empty_check ... ok
test test_full_archive_backup_restore_roundtrip_with_db_snapshot ... ok
Result: 2 passed; 0 failed

Running tests\postgres_least_privilege_test.rs
test test_spine_app_role_has_no_superuser_privileges ... ok
Result: 1 passed; 0 failed

Running tests\profile_enforcement_test.rs
test test_deployment_profile_parsing_and_requirements ... ok
test test_core_forge_disallows_untrusted_workflow_triggers ... ok
test test_core_forge_rejects_untrusted_runner_registration ... ok
test test_isolated_executor_fails_closed_on_untrusted_execution ... ok
Result: 4 passed; 0 failed

Running tests\postgres_live_test.rs
test test_postgres_live_migrations_and_concurrency ... ok
test test_postgres_live_identity_and_auth_lifecycle ... ok
test test_postgres_live_persistence_across_connections ... ok
test test_postgres_live_repository_and_permissions ... ok
Result: 4 passed; 0 failed

Running tests\backup_restore_test.rs
test test_path_traversal_hardening ... ok
test test_restore_tampering_and_corruption_rejection ... ok
test test_backup_creation_and_verified_restoration ... ok
test test_encrypted_backup_creation_and_verified_restoration ... ok
test test_encrypted_backup_key_rotation ... ok
Result: 5 passed; 0 failed
```

---

## 2. Clippy & Linter Verification

```
$ cargo clippy --manifest-path git-alternative/Cargo.toml --all-targets -j 2 -- -D warnings
Finished dev profile [optimized + debuginfo] target(s) in 16.66s (0 errors, 0 warnings)

$ cargo clippy --manifest-path github-alternative/backend/Cargo.toml --all-targets -j 2 -- -D warnings
Finished dev profile [unoptimized + debuginfo] target(s) in 40.77s (0 errors, 0 warnings)

$ cargo fmt --manifest-path git-alternative/Cargo.toml --all -- --check
Passed (0 diffs)

$ cargo fmt --manifest-path github-alternative/backend/Cargo.toml --all -- --check
Passed (0 diffs)

$ npm run build --prefix github-alternative/frontend
vite v5.4.21 building for production...
dist/index.html                   0.49 kB │ gzip:  0.33 kB
dist/assets/index-Cvc-eOIK.css    6.85 kB │ gzip:  1.80 kB
dist/assets/index-C-aBMaF6.js   274.97 kB │ gzip: 69.50 kB
✓ built in 1.76s
```
