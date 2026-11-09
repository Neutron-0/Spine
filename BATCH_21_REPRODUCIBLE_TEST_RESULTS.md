# SPINE — BATCH 21J REPRODUCIBLE TEST RESULTS
## Master Engineering Directive: Distributed Restore Convergence, Database Consistency, Backup Security & Production Integrity

**Date:** 2026-11-09  
**Milestone:** Batch 21J Master Engineering Directive  
**Operating System:** Windows 11  
**Target Profile:** `core-forge-v1` / `release`  

---

### 1. Global Restore Convergence & 15-Point Crash Matrix (`global_restore_convergence_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 0.07s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test global_restore_convergence_test -j 2`
- **Scenarios Verified:**
  1. `test_canonical_seven_participants_contract_validation`: Validates complete 7-participant set (`postgres_data`, `spine_storage`, `node-1`, `node-2`, `node-3`, `coordinator`, `spine_ssh`); enforces unique participant IDs; enforces unique archive paths; rejects canonical volume name substitutions; rejects invalid archive path structures.
  2. `test_crash_matrix_point_1_to_3_pre_decision_aborts_cleanly`: Interruption before global commit decision (`Initialized`, `StagedStorage`, `Prepared`) cleanly cleans up staging directories and purges journals without altering baseline storage generation.
  3. `test_crash_matrix_point_4_decision_write_fallback_to_last_valid`: Power loss or corruption during commit decision write falls back to `.last_valid` journal snapshot without losing restore state.
  4. `test_crash_matrix_point_5_to_9_post_decision_rolls_forward`: Interruption immediately after durable commit decision rolls forward across live storage and participants, updating generation to target generation and achieving full convergence.
  5. `test_crash_matrix_point_14_and_15_recovery_idempotence`: Repeated executions of startup recovery are strictly idempotent, cleanly completing finalization and leaving live storage on target generation.
  6. `test_api_readiness_blocks_during_restore_and_clears_on_convergence`: Active restore journals in progress report `restore_in_progress: true` in readiness probe, returning HTTP 503 until full recovery and finalization complete.

---

### 2. Distributed Participant Integrity & Production Policy (`distributed_participant_integrity_test.rs`)
- **Status:** PASS (3 passed; 0 failed; finished in 1.49s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test distributed_participant_integrity_test -j 2`
- **Scenarios Verified:**
  1. `test_participant_registry_production_enforcement`: Under `core-forge-v1`, complete 7-participant set passes validation; omitting any participant fails closed with `FATAL BACKUP INCOMPLETE`; mismatched fencing generations fail; 0-byte snapshots fail; invalid SHA-256 digests fail; duplicate participant IDs fail.
  2. `test_ssh_host_identity_capture_and_restore_roundtrip`: Host Ed25519 keypair is captured into in-memory snapshot, fingerprint SHA-256 matches `russh` parsed key, restored into target directory, verified identical fingerprint, and tampered fingerprint fails closed with mismatch error.
  3. `test_distributed_cutover_multi_phase_rollback_preservation`: In Phase 4 Publish, live storage is updated while `.old_generation_<id>` is retained. On simulated Phase 5 verification failure, rollback restores original generation data.

---

### 3. Crash-Recoverable Restore & Whole-Generation Cutover (`restore_failure_injection_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 1.51s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test restore_failure_injection_test -j 2`
- **Scenarios Verified:**
  1. `test_startup_crash_recovery_cleans_interrupted_staging`: Pre-decision crash during `StagedStorage` cleanly removes temporary directory and deletes journal without touching live storage.
  2. `test_startup_recovery_rolls_forward_on_interrupted_db_cutover`: Interruption during `DbPublished` phase rolls forward across storage, updating storage generation to target generation and converging on target state.
  3. `test_restore_database_failure_leaves_live_storage_untouched`: Database restore failure during staging rolls back cleanly, leaving existing live repository files completely untouched.
  4. `test_restore_database_success_publishes_storage_atomically`: Successful restore sequences atomically transition `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`, publishing files, writing `.spine_storage_generation`, and recording database restore generation.
  5. `test_whole_generation_cutover_absent_files_do_not_survive`: Staging an archive lacking deleted files atomically moves the live directory to `.old_generation_<id>`, completely eliminating absent files from the live root without leftover artifacts.
  6. `test_multi_volume_cluster_backup_manifest_and_cutover_isolation`: Multi-volume participants verified isolated during cutover and staging.

---

### 4. Production Backup Restore Roundtrip & Generation Monotonicity (`production_backup_restore_roundtrip_test.rs`)
- **Status:** PASS (5 passed; 0 failed; finished in 0.11s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test production_backup_restore_roundtrip_test -j 2`
- **Scenarios Verified:**
  1. `test_fencing_generation_monotonicity_and_fail_closed`: Stale generation restore attempts fail closed.
  2. `test_ssh_identity_staged_publication_and_rollback_preservation`: SSH identity is staged in isolated directory and only published after verification.
  3. `test_forward_recovery_from_storage_published`: Forward recovery at `StoragePublished` cleans up old generation and finalizes to `Committed`.
  4. `test_participant_corruption_and_mismatch_rejection`: Corrupted participant payloads fail checksum validation.
  5. `test_full_cluster_participant_contract_and_streaming_payload`: Captures, stages, and verifies complete multi-volume cluster backups.

---

### 5. SPNEBAK v2 Streaming AEAD Encryption (`backup_v2_streaming_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 2.41s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test backup_v2_streaming_test -j 2`
- **Scenarios Verified:**
  1. `test_spnebak_v2_header_bounds_validation`: Enforces PBKDF2 iteration bounds (10,000 to 1,000,000) and chunk sizes (4 KB to 16 MB).
  2. `test_spnebak_v2_streaming_chunked_roundtrip`: End-to-end encryption and streaming decryption with bounded memory footprint.
  3. `test_spnebak_v2_tampering_and_authentication_rejection`: Corrupted ciphertext and altered authentication tags fail authentication.
  4. `test_spnebak_v2_streaming_key_rotation`: In-place AEAD key rotation re-encrypts archives without intermediate plaintext disk exposure.
  5. `test_consistency_barrier_draining_window`: Consistency barrier drains active writers before backup capture.
  6. `test_spnebak_v2_rejects_trailing_bytes_after_final_manifest`: Trailing bytes or truncation rejected.

---

### 6. Storage Node Persistence & Maintenance Fencing (`distributed_artifact_persistence_test.rs`, `backend::tests`)
- **Status:** PASS (6 passed; 0 failed; finished in 0.05s)
- **Command:** `cargo test --manifest-path git-alternative/apps/spine-storage-node/Cargo.toml -j 1`
- **Scenarios Verified:**
  1. `test_fencing_generation_persistence_and_restore_finalization`: Fencing generation is persisted to `.fencing_generation` via atomic replace; volume restores require active maintenance mode; `.old_restore_<id>` retained after commit and purged on finalize.
  2. `test_storage_node_artifact_persistence_and_restart`: Chunks, manifests, and artifact data survive process shutdown and restart across replica nodes.
  3. `backend::tests::test_storage_node_writer_guard_and_fencing`: Verifies RAII `StorageWriterGuard` tracking, `active_writers` atomic counter, and writer rejection during active maintenance.
  4. Configuration resolution tests from environment variables and CLI arguments.

---

### 7. Workflow Security & Trust Enforcement (`hostile_workflow_test.rs`, `profile_enforcement_test.rs`)
- **Status:** PASS (11 passed; 0 failed)
- **Commands:**
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test hostile_workflow_test -j 2`
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test profile_enforcement_test -j 2`
- **Scenarios Verified:**
  1. Untrusted CI runner registrations and triggers strictly forbidden under `core-forge-v1`.
  2. Workflow execution provenance and repository/job binding validated.
  3. Malformed YAML, cyclical DAGs, process tree timeouts, and secret masking in runner output verified.

---

### 8. Static Analysis & Code Quality
- **`cargo fmt --manifest-path git-alternative/Cargo.toml --all -- --check`:** PASSED (exit code 0, 0 diffs)
- **`cargo fmt --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --all -- --check`:** PASSED (exit code 0, 0 diffs)
- **`cargo clippy -j1 --manifest-path git-alternative/Cargo.toml --all-targets -- -D warnings`:** PASSED (exit code 0, 0 warnings)
- **`cargo clippy -j2 --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --all-targets -- -D warnings`:** PASSED (exit code 0, 0 warnings)

---

### 9. Release Candidate Binary Checksums
```
06038a8050933a8d16f790146a7e5b8c9ff2b438b55d0d3b5e878248f48895de  spine-api.exe
50374c13103fc94dfb20c2d6e2d4ea9b0f613b27d021dfa82dfc49443041fa54  spine-coordinator.exe
a72f02ded65c79c2c6f92963ccefa04b6d4f4e96c461b2724d048d851c97bfdc  spine-storage-node.exe
593ebb979850ecb4d310bd390d5719be18a14fccf1fd9038675d7a1454ef64fa  spine.exe
```
