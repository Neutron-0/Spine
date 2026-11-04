# Batch 19 Cross-System Verification & Regression Report

## 1. Executive Summary

As part of the **Spine Batch 19 Engineering Milestone (Production Packaging, PostgreSQL Readiness, Deployment Architecture, Backup/Recovery, and Operational Rehearsal)**, comprehensive regression and integration verification was executed across the native Spine VCS and Spine Forge subsystems.

Special focus was dedicated to closing the verification gaps identified in Batch 18 (Native Git-over-SSH, Git LFS, File Locking, Strict Host Key Checking, and SSH Server Restart Persistence) and validating the new coordinated backup, disaster recovery, and PostgreSQL migration mechanisms.

---

## 2. Environment Specifications

All verification procedures were performed in the production-candidate environment on native Windows:
- **Operating System:** Windows 11 Enterprise (x86_64)
- **Rust Toolchain:** `rustc 1.97.1` (commit `a929280bf 2026-03-09`) / `cargo 1.97.1`
- **Official OpenSSH Client:** `OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2`
- **Official Git Client:** `git version 2.53.0.windows.1`
- **Official Git LFS Client:** `git-lfs/3.7.1 (GitHub; windows amd64)`

---

## 3. Native Git-over-SSH Verification Suite

The native Git-over-SSH subsystem (`spine-net` + `spine-api::ssh`) was evaluated against real client workflows via official OpenSSH and Git binaries.

### 3.1 Strict Host Key Checking & Advanced Operations
Test file: `github-alternative/backend/apps/spine-api/tests/git_ssh_test.rs`
- **Command:** `cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_ssh_test test_git_ssh_strict_host_key_and_advanced_ops`
- **Execution Parameters:**
  - `GIT_SSH_COMMAND=ssh -o StrictHostKeyChecking=yes -o UserKnownHostsFile=<isolated_hosts> -o IdentityFile=<client_key>`
- **Workflow Steps Verified:**
  1. Automated retrieval of server host public key and injection into isolated `known_hosts` file.
  2. Initial `git clone ssh://git@127.0.0.1:<port>/owner/repo.git`.
  3. Commit and `git push origin main`.
  4. New branch creation `feature-ssh-branch` and `git push origin feature-ssh-branch`.
  5. Annotated tag creation `v1.0.0-ssh` and `git push origin v1.0.0-ssh`.
  6. Second client clone from fresh directory and `git pull`.
  7. Strict Host Key Tampering: modifying the stored fingerprint in `known_hosts` immediately aborts SSH connection with exit code 255 ("REMOTE HOST IDENTIFICATION HAS CHANGED").
- **Result:** **PASSED** (0 failures).

### 3.2 SSH Server Restart & Key Persistence
- **Command:** `cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_ssh_test test_git_ssh_persistence_and_restart`
- **Workflow Steps Verified:**
  1. Boot server 1 with persistent `ssh/host_key_ed25519`.
  2. Capture host public key fingerprint: `SHA256:4t7a1cK...`.
  3. Register client public key in DB.
  4. Client commits and pushes to server 1.
  5. Shut down server 1.
  6. Start server 2 pointing to the exact same storage root and persistent host key.
  7. Verify server 2 loads the existing host key; client connects with unchanged `known_hosts` without warning or error.
  8. Client clones from server 2 and verifies commit log matches.
  9. Key Revocation Test: delete client public key from database. Client attempt to push to server 2 is immediately rejected with `Permission denied (publickey)`.
- **Result:** **PASSED** (0 failures).

### 3.3 Summary of Full SSH Suite
- Total tests executed: 7
  - `test_git_ssh_clone_empty_repo`: PASSED
  - `test_git_ssh_push_and_clone`: PASSED
  - `test_git_ssh_auth_rejection`: PASSED
  - `test_git_ssh_deploy_key_read_only`: PASSED
  - `test_git_ssh_lfs_authenticate`: PASSED
  - `test_git_ssh_strict_host_key_and_advanced_ops`: PASSED
  - `test_git_ssh_persistence_and_restart`: PASSED
- Total elapsed time: 8.55 seconds.

---

## 4. Coordinated Backup & Disaster Recovery Verification

Test file: `github-alternative/backend/apps/spine-api/tests/backup_restore_test.rs`
- **Command:** `cargo test --manifest-path github-alternative/backend/Cargo.toml --test backup_restore_test`
- **Verifications:**
  1. `test_backup_creation_and_verified_restoration`:
     - Creates multi-layer repository tree (`repos/org/project.spine`), object packfiles (`pack-test.spack`), release binary artifacts (`artifacts/releases/v1/release-v1.0.0.tar.gz`), and database metadata snapshot JSON.
     - Creates compressed archive `backup.tar.gz`.
     - Validates manifest: `integrity_status == "VERIFIED"`, `total_files >= 4`, cryptographic SHA-256 for each stored entry.
     - Restores archive into fresh directory. Validates `report.integrity_verified == true`, restored files match source byte-for-byte.
  2. `test_restore_tampering_and_corruption_rejection`:
     - Creates valid backup archive.
     - Deliberately flips bytes within archive payload to simulate disk block corruption.
     - Validates that `BackupManager::restore_backup` detects corruption and terminates with `Integrity verification failed` error, leaving target directory intact.
- **Result:** **PASSED** (2/2 passed, 0.06s).

---

## 5. PostgreSQL Readiness Gate & Migration Suite

- Embedded migrations: 11 sequential, idempotent migration files (`0001_core_identity.sql` through `0011_security_center.sql`).
- Embedded runner: `spine_api::db::run_migrations` using transactional DDL and `_schema_migrations` tracking table.
- Runtime status: Because local Docker daemon is offline (`npipe:////./pipe/dockerDesktopLinuxEngine` unavailable) and PostgreSQL 16 is not installed locally on this host, live PostgreSQL execution remains marked **BLOCKED** in `docs/BATCH_19_DEPLOYMENT_BLOCKER_REGISTER.md`. All schemas, migration logic, and container specifications are fully authored and ready for target environments.
