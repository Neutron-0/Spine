# SPINE — BATCH 21F REPRODUCIBLE TEST RESULTS
## Multi-Volume Backup, Fencing Coordination, Crash-Safe Restore and Release Verification Evidence

**Date:** 2026-11-08  
**Target HEAD:** Batch 21F deployment readiness candidate  
**Operating System:** Windows 11 / PostgreSQL 16  
**Rust Version:** rustc 1.88.0-nightly / stable toolchain compatible (MSRV 1.88)  

---

### 1. Crash-Recoverable Restore & Whole-Generation Cutover (`restore_failure_injection_test.rs`)
- **Status:** PASS (6 passed; 0 failed; finished in 1.95s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test restore_failure_injection_test --offline`
- **Scenarios Verified:**
  1. `test_startup_crash_recovery_cleans_interrupted_staging`: Pre-publication crash during `StagedStorage` cleanly removes temporary directory and deletes journal without touching live storage.
  2. `test_startup_recovery_fails_closed_on_interrupted_db_cutover`: Interruption during `DbPublished` phase (database updated, storage not yet published) triggers fatal `handle_startup_recovery` failure, blocking server startup and preventing split-brain corruption.
  3. `test_restore_database_failure_leaves_live_storage_untouched`: Database restore failure during staging rolls back cleanly, leaving existing live repository files completely untouched.
  4. `test_restore_database_success_publishes_storage_atomically`: Successful restore sequences atomically transition `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`, publishing files, writing `.spine_storage_generation`, and recording database restore generation.
  5. `test_whole_generation_cutover_eliminates_absent_files`: Staging an archive lacking deleted files atomically archives the live directory to `.old_generation_<id>`, completely eliminating files deleted in the snapshot from the live root without leftover artifacts.
  6. `test_storage_node_fencing_rejects_stale_generation`: Mutating RPCs (`commit_pack`) sent to storage nodes with a stale fencing generation or during active maintenance barrier are strictly rejected with `StaleFencingGeneration` / `InMaintenanceBarrier`.

---

### 2. Multi-Volume Cluster Consensus & Storage Node RPCs (`spine-cluster` and `git-alternative`)
- **Status:** PASS (103 passed; 0 failed; 0 ignored; finished in 4.12s)
- **Command:** `cargo test --manifest-path git-alternative/Cargo.toml --offline`
- **Scenarios Verified:**
  1. `FileBackedCoordinatorStore`: Maintenance barrier entry, exit, state query, memory snapshot capture, and staged restoration commit/rollback.
  2. `FileSystemStorageBackend`: Active writer tracking, maintenance barrier gating, monotonic fencing generation validation on pack commit, in-memory tar.gz snapshot creation, and whole-generation rollback/commit.
  3. Protocol message serialization and deserialization for all new cluster RPCs: `EnterMaintenanceBarrier`, `ExitMaintenanceBarrier`, `QueryMaintenanceStatus`, `CaptureVolumeSnapshot`, `StageVolumeRestore`, `CommitVolumeRestore`, `RollbackVolumeRestore`.

---

### 3. Disposable Database Safety Contract (`test_database_safety_test.rs`)
- **Status:** PASS (1 passed; 0 failed; finished in 0.00s)
- **Command:** `cargo test --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml --test test_database_safety_test --offline`
- **Scenarios Verified:**
  1. Opt-in flag enforcement: Refuses destructive modifications without `SPINE_ALLOW_DESTRUCTIVE_TESTS=1`.
  2. Pre-existing database protection: Rejects pre-existing database targets missing current harness proof.
  3. Fail-closed provisioning: Network or unreachable database configuration fails immediately without performing SQL operations.
  4. Harness identity isolation: Silent fallback from `app_pool` to `migrator_pool` prohibited; least-privilege connection required.

---

### 4. Authoritative Workflow Provenance (`hostile_workflow_test.rs`, `profile_enforcement_test.rs`, `failure_injection_test.rs`)
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

### 5. SPNEBAK v2 Streaming Encryption & Backup Recovery (`backup_v2_streaming_test.rs`, `backup_restore_test.rs`)
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

### 6. Workspace Static Analysis & Code Quality
- **`cargo fmt --check` (git-alternative):** PASSED (0 diffs)
- **`cargo fmt --check` (spine-api):** PASSED (0 diffs)
- **`cargo clippy --all-targets -- -D warnings` (git-alternative):** PASSED (0 warnings)
- **`cargo clippy --all-targets -- -D warnings` (spine-api):** PASSED (0 warnings)

---

### 7. Release Candidate Artifact Inventory & SHA-256 Checksums
All 4 release binaries compiled successfully with optimized release profiles:

| Binary Artifact | Size (bytes) | SHA-256 Checksum |
|---|---|---|
| `spine-api.exe` | 22,028,288 | `d27246f9f66dffcfb3d76bf8b0edf7eb66ee82aa0bec9ed747aeadb616b58261` |
| `spine-coordinator.exe` | 2,514,432 | `6ca2d304cda19698a79045622909edfb752b6ff7b44e8c84c2405a9df5594681` |
| `spine-storage-node.exe` | 2,293,248 | `1a590d79c307d288da8ebf87840ee8a43216cc39e8abbeea86556eff04d23ddc` |
| `spine.exe` | 8,717,312 | `e639bdd7d3ef8888bfbf77534d6ad15d45b61c52fa14c8a7551e27268f3ee2a8` |

---

### 8. Environmental Reality & Live Daemon Status Disclosure
- **WSL Service Status:** Inactive (`Wsl/Service/E_UNEXPECTED`).
- **Docker Daemon Status:** Offline (`open //./pipe/dockerDesktopLinuxEngine: The system cannot find the file specified`).
- **Audit Verification Status:** Migration execution and container startup against live daemons are honestly reported as **NOT VERIFIED** due to host service offline state. In accordance with Directive Sections 1, 6, 7 & 10, host Docker services and persistent WSL PostgreSQL installations were never modified or altered.
