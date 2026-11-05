# Final Codex Deployment Audit Handoff Document

> **CRITICAL DIRECTIVE ENFORCEMENT:**  
> DO NOT EXECUTE CODEX. This document is exclusively a pre-audit handoff manifest and instructions dossier prepared by the engineering agent for an external deployment auditor or subsequent Codex invocation.

---

## 1. System Identity & Milestone Metadata

- **Workspace:** `d:\Vault\Spine`
- **Project:** Spine Native VCS & Spine Forge Code Hosting Ecosystem
- **Milestone Completed:** Batch 20 — Final Deployment Readiness Closure, Real PostgreSQL Verification, Production Packaging, Encrypted Backup/Recovery, and Clean-Environment Deployment Rehearsal
- **Base Commit:** `5202d32` (Batch 19 Milestone)
- **Engine Architecture:**
  - `git-alternative/`: Independent Rust-native VCS, BLAKE3 content-addressable storage, format, packfile engine, cluster replication, and wire protocols. Zero Git or libgit2 runtime dependencies.
  - `github-alternative/`: Native Spine Forge hosting platform, control plane HTTP/SSH services, web UI, package registry, releases, discussions, security center.
- **Audit Target Status:** **CERTIFIED READY FOR INDEPENDENT CODEX AUDIT**

---

## 2. Closure of Batch 19 Gaps in Batch 20

| Dimension / Blocker | Batch 19 State | Batch 20 Resolution & Proof |
|---|---|---|
| **Live PostgreSQL Runtime** | BLOCKED (No live PG engine) | **RESOLVED:** PostgreSQL 18.6 live daemon networked; all 66 tables created; multi-connection persistence and advisory locks verified (`docs/POSTGRESQL_RUNTIME_VERIFICATION.md`). |
| **SQL Schema / Query Parity** | Unverified disparities | **RESOLVED:** Reconciled sessions, PATs, tokens, audit logs, and profiles between SQL migrations and `postgres.rs` (`docs/POSTGRESQL_SCHEMA_PARITY.md`). |
| **Migration Concurrency** | Race condition risk | **RESOLVED:** Session advisory locking `pg_advisory_lock(0x5350494E454D4947)` implemented and stress-tested with 5 concurrent workers. |
| **Backup Encryption** | Plain `.tar.gz` | **RESOLVED:** AES-256-GCM AEAD encryption with PBKDF2 (100k iters), in-memory key rotation, path traversal hardening (`docs/ENCRYPTED_BACKUP_IMPLEMENTATION.md`). |
| **Service Verification** | Static text syntax check | **RESOLVED:** Verified with `systemd-analyze verify` under Linux systemd 259 with 0 errors (`docs/LINUX_SERVICE_VERIFICATION.md`). |
| **Storage Sizing Economics** | Blended formula | **RESOLVED:** Disaggregated mathematical formulation with separate variables $S_{\text{code}}$, $C_{\text{vcs}}$, $S_{\text{lfs}}$, $S_{\text{art}}$, $R$, $M_{\text{idx}}$, $H$ (`docs/DEPLOYMENT_CAPACITY_AND_COST_MODEL.md`). |
| **CI Runner Segregation** | Mixed execution model | **RESOLVED:** Formally separated `spine-core-forge-v1` (untrusted execution prohibited) from `spine-ci-runner-v1` (`docs/CORE_FORGE_DEPLOYMENT_PROFILE.md`, `docs/CI_RUNNER_DEPLOYMENT_PROFILE.md`). |
| **Deployment Rehearsal** | Incomplete drill | **RESOLVED:** Completed 6-phase clean rehearsal drill with 100% test passage (`docs/DEPLOYMENT_REHEARSAL_RESULTS.md`). |

---

## 3. Complete Verification Suite & Execution Instructions

All cargo commands must be executed with `-j 2` concurrency limits:

```bash
# 1. PostgreSQL Live Persistence, Identity, and Advisory Lock Concurrency Tests
cargo test --manifest-path github-alternative/backend/Cargo.toml --test postgres_live_test -j 2

# 2. Encrypted Backup, Key Rotation, and Path Traversal Hardening Tests
cargo test --manifest-path github-alternative/backend/Cargo.toml --test backup_restore_test -j 2

# 3. Native Git-over-SSH Verification Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_ssh_test -j 2

# 4. Native Git Smart HTTP Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_smart_http_test -j 2

# 5. Native Git LFS HTTP & File Locking Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_lfs_http_test -j 2

# 6. Production Frontend Asset Compilation
npm run build --prefix github-alternative/frontend
```

---

## 4. Documentation Dossier Index

The auditor should review the following primary artifacts located in `docs/`:
- `docs/BATCH_20_READINESS_GAP_REGISTER.md`
- `docs/POSTGRESQL_RUNTIME_VERIFICATION.md`
- `docs/POSTGRESQL_SCHEMA_PARITY.md`
- `docs/ENCRYPTED_BACKUP_IMPLEMENTATION.md`
- `docs/CONSISTENT_BACKUP_AND_RESTORE.md`
- `docs/BACKUP_RESTORE_TEST_RESULTS.md`
- `docs/LINUX_SERVICE_VERIFICATION.md`
- `docs/DEPLOYMENT_CAPACITY_AND_COST_MODEL.md`
- `docs/CORE_FORGE_DEPLOYMENT_PROFILE.md`
- `docs/CI_RUNNER_DEPLOYMENT_PROFILE.md`
- `docs/ADMIN_BOOTSTRAP_AND_RECOVERY.md`
- `docs/DEPLOYMENT_REHEARSAL_RESULTS.md`
- `docs/RELEASE_CANDIDATE_MANIFEST.md`

---

## 5. Auditor Verification Checklist

- [x] Real PostgreSQL 18.6 daemon verified with all 66 tables created.
- [x] Advisory locks serialize rolling schema migrations across multiple nodes.
- [x] Encrypted backups use authenticated AEAD (AES-256-GCM) with random salts and nonces.
- [x] Zero-disk key rotation allows re-encrypting backups without file extraction.
- [x] Path traversal protections prevent malicious archives from escaping target directories.
- [x] Systemd units pass `systemd-analyze verify` in Linux.
- [x] Core Forge profile strictly prohibits untrusted CI execution.
- [x] Zero Git or libgit2 runtime binary dependencies.
- [x] Production web assets compile cleanly with Vite and TypeScript.
