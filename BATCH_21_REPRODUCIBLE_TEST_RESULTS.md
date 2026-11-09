# SPINE — BATCH 21G REPRODUCIBLE TEST RESULTS
## Multi-Volume Backup Completeness, SSH Host Identity Preservation, Distributed 6-Phase Cutover, Persistent Fencing and Release Verification Evidence

**Date:** 2026-11-09  
**Target HEAD:** Batch 21G deployment readiness candidate  
**Operating System:** Windows 11 / PostgreSQL 16  
**Rust Version:** rustc 1.88.0-nightly / stable toolchain compatible (MSRV 1.88)  

---

### 1. Distributed Participant Integrity & Production Policy (`distributed_participant_integrity_test.rs`)
- **Status:** PASS (3 passed; 0 failed; finished in 1.43s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test distributed_participant_integrity_test --offline`
- **Scenarios Verified:**
  1. `test_participant_registry_production_enforcement`: Under `core-forge-v1`, complete 5-participant cluster set (`node-1`, `node-2`, `node-3`, `coordinator`, `spine_ssh`) passes; omitting any single participant fails closed with `FATAL BACKUP INCOMPLETE`; mismatched fencing generations fail; 0-byte snapshots fail; invalid SHA-256 digests fail; duplicate participant IDs fail. Under `development` profile, partial participants are permitted.
  2. `test_ssh_host_identity_capture_and_restore_roundtrip`: Host keypair is captured into in-memory snapshot, fingerprint SHA-256 matches `russh` parsed key, restored into target directory, verified identical fingerprint, and tampered fingerprint fails closed with mismatch error.
  3. `test_distributed_cutover_multi_phase_rollback_preservation`: In Phase 4 Publish, live storage is updated while `.old_generation_<id>` is strictly retained. On simulated Phase 5 verification failure, distributed rollback cleanly restores original generation data.

---

### 2. Crash-Recoverable Restore & Whole-Generation Cutover (`restore_failure_injection_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 1.44s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test restore_failure_injection_test --offline`
- **Scenarios Verified:**
  1. `test_startup_crash_recovery_cleans_interrupted_staging`: Pre-publication crash during `StagedStorage` cleanly removes temporary directory and deletes journal without touching live storage.
  2. `test_startup_recovery_fails_closed_on_interrupted_db_cutover`: Interruption during `DbPublished` phase triggers fatal `handle_startup_recovery` failure, blocking server startup and preventing split-brain corruption.
  3. `test_restore_database_failure_leaves_live_storage_untouched`: Database restore failure during staging rolls back cleanly, leaving existing live repository files completely untouched.
  4. `test_restore_database_success_publishes_storage_atomically`: Successful restore sequences atomically transition `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`, publishing files, writing `.spine_storage_generation`, and recording database restore generation.
  5. `test_whole_generation_cutover_absent_files_do_not_survive`: Staging an archive lacking deleted files atomically archives the live directory to `.old_generation_<id>`, completely eliminating absent files from the live root without leftover artifacts.
  6. `test_multi_volume_cluster_backup_manifest_and_cutover_isolation`: Multi-volume participants verified isolated during cutover and staging.

---

### 3. Persistent Fencing Generation & Storage Node Backend (`distributed_artifact_persistence_test.rs`)
- **Status:** PASS (2 passed; 0 failed; finished in 0.05s)
- **Command:** `cargo test --manifest-path git-alternative/apps/spine-storage-node/Cargo.toml --offline`
- **Scenarios Verified:**
  1. `test_fencing_generation_persistence_and_restore_finalization`: Fencing generation is persisted to `.fencing_generation` on disk via atomic tempfile rename. Commit volume restore retains `.old_restore_<snapshot_id>`. Finalize volume restore purges `.old_restore_<snapshot_id>`. Upon backend restart on the same directory, fencing generation is reloaded from disk and never regresses.
  2. `test_storage_node_artifact_persistence_and_restart`: Chunks, manifests, and artifact data survive process shutdown and restart across replica nodes.

---

### 4. Multi-Volume Cluster Consensus & Storage Node RPCs (`git-alternative` Workspace)
- **Status:** PASS (103 passed; 0 failed; finished in 4.19s)
- **Command:** `cargo test --manifest-path git-alternative/Cargo.toml --offline`
- **Scenarios Verified:**
  1. `FileBackedCoordinatorStore`: Persistent `.coordinator_fencing_generation` reloaded on boot; maintenance barrier entry/exit, state query, memory snapshot capture, and staged restoration commit/rollback/finalize.
  2. `FileSystemStorageBackend`: Active writer tracking, maintenance barrier gating, monotonic fencing generation validation on pack commit, in-memory tar.gz snapshot creation, and whole-generation rollback/commit/finalize.
  3. Protocol message serialization and deserialization for all cluster RPCs including `FinalizeVolumeRestore`.

---

### 5. Disposable Database Safety Contract (`test_database_safety_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 0.00s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test test_database_safety_test --offline`
- **Scenarios Verified:**
  1. Opt-in flag enforcement: Refuses destructive modifications without `SPINE_ALLOW_DESTRUCTIVE_TESTS=1`.
  2. Pre-existing database protection: Rejects pre-existing database targets missing current harness proof.
  3. Fail-closed provisioning: Network or unreachable database configuration fails immediately without performing SQL operations.
  4. Harness identity isolation: Silent fallback from `app_pool` to `migrator_pool` prohibited; least-privilege connection required.

---

### 6. Authoritative Workflow Provenance (`hostile_workflow_test.rs`, `profile_enforcement_test.rs`, `failure_injection_test.rs`)
- **Status:** PASS (16 passed; 0 failed)
- **Commands:**
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test hostile_workflow_test --offline`
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test profile_enforcement_test --offline`
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test failure_injection_test --offline`
- **Scenarios Verified:**
  1. Authoritative trust token validation in production execution path (`RunnerService::acquire_job` and `complete_job`).
  2. Elimination of repository ID bypass in `IsolatedRunnerExecutor`.
  3. Strict rejection of untrusted workflow execution under `core-forge-v1` deployment profile.
  4. Runner mutation endpoints participate in global consistency barrier (`acquire_write_permit` and `verify_active`).

---

### 7. SPNEBAK v2 Streaming Encryption & Backup Recovery (`backup_v2_streaming_test.rs`, `backup_restore_test.rs`)
- **Status:** PASS (11 passed; 0 failed)
- **Commands:**
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test backup_v2_streaming_test --offline`
  - `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test backup_restore_test --offline`
- **Scenarios Verified:**
  1. Authenticated streaming chunked encryption and decryption roundtrip (AES-256-GCM).
  2. Header tampering and invalid authentication tag rejection.
  3. In-place master key rotation without intermediate plaintext exposure.
  4. End-to-end backup archive creation with database dump, manifests, and participants.
  5. Monotonic fencing generation embedded in backup manifest.

---

### 8. Workspace Static Analysis & Code Quality
- **`cargo fmt --check` (git-alternative):** PASSED (0 diffs)
- **`cargo fmt --check` (spine-api):** PASSED (0 diffs)
- **`cargo clippy --all-targets -- -D warnings` (git-alternative):** PASSED (0 warnings)
- **`cargo clippy --all-targets -- -D warnings` (spine-api):** PASSED (0 warnings)

---

### 9. Release Candidate Artifact Inventory & SHA-256 Checksums
All 4 release binaries compiled successfully with optimized release profiles:

| Binary Artifact | Size (bytes) | SHA-256 Checksum |
|---|---|---|
| `spine-api.exe` | 22,036,480 | `5e6c91e7931b0ffd4f278d7c6c023ff0c1a50fa8b9cff7090461c2da912fea00` |
| `spine-coordinator.exe` | 2,514,432 | `fa149656e7f65557fe0a566dd534cde0f6874660accadd15fd56d9a1b71c87de` |
| `spine-storage-node.exe` | 2,293,760 | `5e875c96bdfc82e924e381ff3852d71a2ecf35d1f3114811db93fcabef265dbd` |
| `spine.exe` | 8,717,312 | `8e094c542d7350be8671f7d5d76303b0ed6e0274aa72161792700bc8d244b788` |

---

### 10. Environmental Reality & Live Daemon Status Disclosure
- **WSL Service Status:** Inactive (`Wsl/Service/E_UNEXPECTED`).
- **Docker Daemon Status:** Offline (`open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified`).
- **Audit Verification Status:** Migration execution and container startup against live daemons are honestly reported as **NOT VERIFIED** due to host service offline state. In accordance with Directive Sections 1, 6, 7 & 10, host Docker services and persistent WSL PostgreSQL installations were never modified or altered.
