# Final Codex Deployment Audit Handoff Document

> **CRITICAL DIRECTIVE ENFORCEMENT:**
> DO NOT EXECUTE CODEX. This document is exclusively a pre-audit handoff manifest and instructions dossier prepared by the engineering agent for an external deployment auditor or subsequent Codex invocation.

---

## 1. System Identity & Milestone Metadata

- **Workspace:** `d:\Vault\Spine`
- **Project:** Spine Native VCS & Spine Forge Code Hosting Ecosystem
- **Milestone Completed:** Batch 19 — Production Packaging, PostgreSQL Readiness, Deployment Architecture, Backup/Recovery, and Operational Rehearsal
- **Base Commit:** `b6881c6` (Batch 18 Milestone)
- **Engine Architecture:**
  - `git-alternative/`: Independent Rust-native VCS, BLAKE3 content-addressable storage, format, packfile engine, cluster replication, and wire protocols.
  - `github-alternative/`: Native Spine Forge hosting platform, control plane HTTP/SSH services, web UI, package registry, releases, discussions, security center.

---

## 2. Inventory of Delivered Batch 19 Capabilities

1. **Subsystem Readiness & Blocker Inventory:**
   - 35 subsystems fully evaluated and registered in `docs/BATCH_19_DEPLOYMENT_READINESS_REGISTER.md`.
   - Comprehensive blocker classification in `docs/BATCH_19_DEPLOYMENT_BLOCKER_REGISTER.md`.
2. **Native Git-over-SSH Hardening & Persistence:**
   - Verified strict host key checking (`StrictHostKeyChecking=yes`, isolated `known_hosts`) with official OpenSSH 9.5p2 client.
   - Tested full Git lifecycle over SSH: clone, fetch, pull, push, branch push, tag push, host key mismatch rejection.
   - Verified persistent Ed25519 host key continuity across server restarts and immediate push rejection upon key revocation.
3. **PostgreSQL Migration Engine & Relational Schema:**
   - 11 sequential, idempotent SQL migration files (`0001` to `0011`) covering 60+ tables.
   - Embedded transactional migration runner `spine_api::db::run_migrations`.
   - Automated startup migration via `PgDatabase::connect_and_migrate`.
4. **Coordinated Backup & Disaster Recovery:**
   - `BackupManager` in `spine_api::backup` creating and restoring SHA-256 verified `.tar.gz` archives with cryptographic `manifest.json`.
   - Rejection of tampered or corrupted backup archives.
5. **One-Time Admin Bootstrap & CLI Subcommands:**
   - Subcommands `serve`, `migrate`, `bootstrap-admin`, `backup create`, `backup restore`.
   - Admin bootstrap with 32-character secure random password, Argon2id hashing, and single-execution lockout.
6. **Production Packaging & Hardening:**
   - Multi-stage `Dockerfile.production` and `Dockerfile.api` with unprivileged `spine` user (UID 10001).
   - Production `docker-compose.yml` with PostgreSQL 16, healthchecks, and persistent volumes.
   - Hardened `systemd` service units (`spine-api.service`, `spine-coordinator.service`, `spine-storage-node.service`).
   - `.env.example` with documented configuration parameters.

---

## 3. Current Verification Status & Active Blockers

| Subsystem / Capability | Verification Status | Notes / Location |
|---|---|---|
| **Git-over-SSH (Client & Server)** | **VERIFIED** | 7/7 tests passed (`git_ssh_test.rs`) |
| **Backup & Disaster Recovery** | **VERIFIED** | 2/2 tests passed (`backup_restore_test.rs`) |
| **Git Smart HTTP & Git LFS** | **VERIFIED** | Passed in Batch 16-18 test suites |
| **Frontend React / TypeScript UI** | **VERIFIED** | Compiled with 0 errors via Vite |
| **Embedded SQL Migrations** | **VERIFIED (Code)** | 11 SQL scripts compiled into binary |
| **Live PostgreSQL Execution** | **BLOCKED** | Local Docker daemon offline; native PostgreSQL not installed. Documented in `docs/BATCH_19_DEPLOYMENT_BLOCKER_REGISTER.md` (BLK-01). |

---

## 4. How to Execute the Verification Suite

All cargo commands must be executed with `-j 2` concurrency limits:

```bash
# 1. Run Backup & Restore Verification Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test backup_restore_test -j 2

# 2. Run Native Git-over-SSH Verification Suite (requires OpenSSH and git)
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_ssh_test -j 2

# 3. Run Native Git Smart HTTP Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_smart_http_test -j 2

# 4. Run Native Git LFS Suite
cargo test --manifest-path github-alternative/backend/Cargo.toml --test git_lfs_http_test -j 2

# 5. Build Frontend Web Application
npm run build --prefix github-alternative/frontend
```

---

## 5. Instructions for Final Deployment Audit (Codex)

When the external auditor or Codex deployment review is scheduled:
1. Ensure a live PostgreSQL 16 instance is reachable.
2. Provide `DATABASE_URL=postgres://spine_app:password@<host>:5432/spine_forge`.
3. Run `spine-api migrate` to apply all 11 schema migrations.
4. Run live PostgreSQL integration tests to clear blocker BLK-01.
5. Review the comprehensive documentation dossier in `docs/`:
   - `docs/BATCH_19_DEPLOYMENT_READINESS_REGISTER.md`
   - `docs/BATCH_19_DEPLOYMENT_BLOCKER_REGISTER.md`
   - `docs/BATCH_19_CROSS_SYSTEM_VERIFICATION.md`
   - `docs/PRODUCTION_STORAGE_ARCHITECTURE.md`
   - `docs/BACKUP_ARCHITECTURE.md`
   - `docs/DISASTER_RECOVERY_PLAN.md`
   - `docs/RESTORE_RUNBOOK.md`
   - `docs/PRODUCTION_SECRET_MANAGEMENT.md`
   - `docs/PRODUCTION_NETWORK_ARCHITECTURE.md`
   - `docs/PRODUCTION_SERVICE_PACKAGING.md`
   - `docs/POSTGRESQL_DEPLOYMENT.md`
   - `docs/DEPLOYMENT_CAPACITY_AND_COST_MODEL.md`
   - `docs/PRODUCTION_OBSERVABILITY.md`
   - `docs/INCIDENT_RESPONSE_RUNBOOK.md`
   - `docs/OPERATIONS_MANUAL.md`
   - `docs/UPGRADE_AND_ROLLBACK.md`
   - `docs/RELEASE_INTEGRITY.md`
   - `docs/DEPLOYMENT_REHEARSAL.md`
