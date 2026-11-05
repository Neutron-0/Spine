# Batch 20 Readiness Gap Register

## 1. Executive Summary & Assessment Methodology

Following the **Batch 20 Engineering Directive**, this register documents the empirical verification status of all 16 core deployment dimensions of the Spine VCS and Spine Forge ecosystem.

Every capability is classified into one of five rigorous states:
- **CLOSED:** Fully implemented, verified against live operational environments, and backed by automated tests.
- **PARTIALLY CLOSED:** Implementation complete, partial operational proof achieved, specific hardening or verification steps remaining.
- **OPEN:** Architecture defined or partially implemented, but requires completion in Batch 20.
- **BLOCKED:** Cannot be completed due to external environment or dependency blockers with documented remediation.
- **FAILED:** Execution attempted and produced defect or failure requiring immediate engineering fix.

---

## 2. Deployment Dimension Inventory & Verification Matrix

| ID | Subsystem / Requirement | Batch 19 Baseline | Batch 20 Reality & Evidence | Current State | Required Evidence to Close |
|---|---|---|---|---|---|
| **GAP-01** | **Live PostgreSQL Persistence** | BLOCKED (Docker offline, no local DB) | PostgreSQL 18.6 active in WSL2 Ubuntu (`172.18.77.29:5432`), 11 sequential migrations applied to `spine_forge`. | **PARTIALLY CLOSED** | Run comprehensive live functional and concurrent test suite (`PgDatabase`). |
| **GAP-02** | **Migration Startup Safety & Locks** | Automatic migration in `serve` without distributed lock | Migrations run inside SQL transaction, but lack Postgres advisory lock for multi-instance concurrency. | **OPEN** | Add `pg_advisory_lock` / `pg_try_advisory_lock` in `run_migrations` and verify concurrent startup. |
| **GAP-03** | **SQL Query / Schema Parity** | Queries in `postgres.rs` not tested against real live DB | All 66 tables created, need automated query parity verification across all 60+ tables. | **OPEN** | Schema parity inspection and live query test exercising CRUD on every model. |
| **GAP-04** | **Authenticated Backup Encryption** | Unencrypted `.tar.gz` with SHA-256 hash manifest | No AEAD ciphertext encryption at rest. Keys not versioned or rotatable. | **OPEN** | Implement ChaCha20-Poly1305 / AES-256-GCM AEAD encryption, key versioning, rotation test. |
| **GAP-05** | **Cross-System Backup Consistency** | Local storage tar + optional JSON dump | No coordinated write-barrier or concurrent push fencing during backup creation. | **OPEN** | Implement snapshot barrier, coordinated PostgreSQL WAL / dump + immutable packfile snapshot. |
| **GAP-06** | **Backup Archive Hardening** | Basic file extraction | Untrusted archive inputs must be protected against directory traversal, symlink escapes, zip bombs. | **OPEN** | Hardened sanitization for archive member paths, quota limits, and canonical target verification. |
| **GAP-07** | **Full Restoration Verification** | 2 synthetic tests | Restored into tempdir, but did not exercise full live service restart, clone, push, LFS, and Cargo dependencies. | **PARTIALLY CLOSED** | Comprehensive end-to-end disaster recovery rehearsal from restored backup. |
| **GAP-08** | **Production Container Packaging** | Dockerfiles authored, unbuilt | Docker daemon offline on host; WSL Ubuntu container tooling available. | **OPEN** | Build container images, validate non-root user, multi-stage layers, and healthcheck. |
| **GAP-09** | **Linux Systemd Service Units** | Unit files authored, unverified | WSL2 Ubuntu running systemd 259 (`systemd-analyze` available). | **PARTIALLY CLOSED** | Run `systemd-analyze verify`, install units, test process sandboxing in WSL systemd. |
| **GAP-10** | **Deployment Profiles (Core Forge vs CI)** | Environment toggle `UNTRUSTED_WORKFLOW_RUNNER_ENABLED` | Explicit enforcement at API routes and frontend UI required to reject untrusted runner registration. | **PARTIALLY CLOSED** | Verify hard route rejection and disabled frontend controls when in Core Forge profile. |
| **GAP-11** | **Network & TLS Ingress Rules** | Nginx reverse proxy template authored | Canonical origin enforcement, X-Forwarded headers validation, and TLS termination documentation. | **CLOSED** | Validated Nginx streaming SSH and HTTP proxy configuration. |
| **GAP-12** | **SSH Host Identity & Key Management** | Strict host key checking verified | 7/7 tests passed with official OpenSSH client, Ed25519 host key continuity across restarts proven. | **CLOSED** | Test suite `git_ssh_test.rs` passing 100%. |
| **GAP-13** | **Admin Bootstrap & Account Recovery** | CLI subcommand `bootstrap-admin` | Single-execution lockout, 32-char secure random password, Argon2id, audit event. | **PARTIALLY CLOSED** | Document explicit out-of-band host administrator recovery process. |
| **GAP-14** | **Storage Economics & Capacity Formula** | Modeled 200 GB as 840 GB (1x), 1.68 TB (2x), 2.52 TB (3x) | Formula lumped user data, LFS, artifacts, DB, and headroom into single combined multiplier. | **OPEN** | Disaggregate into clean formula with independent variables per Section 10. |
| **GAP-15** | **Developer Lifecycle Rehearsal** | Basic smoke tests | Complete clean-environment lifecycle from user creation to LFS locking, release, and Cargo publish. | **PARTIALLY CLOSED** | Execute comprehensive deployment rehearsal script against live PostgreSQL and services. |
| **GAP-16** | **Codex Audit Readiness Gate** | Pre-audit handoff authored | Blocked on real PostgreSQL integration proof, encrypted backup, and complete recovery drill. | **BLOCKED** | Resolve GAP-01 through GAP-07 before declaring readiness. |

---

## 3. Immediate Action Plan

1. **PostgreSQL Migration Safety & Advisory Lock (GAP-02):** Implement PostgreSQL session advisory lock (`pg_advisory_lock(0x5350494E45)`) in `run_migrations` to serialize multi-instance API startups.
2. **PostgreSQL Schema Parity & Live Integration Test Suite (GAP-01, GAP-03):** Author `postgres_live_test.rs` verifying every entity, transaction boundary, foreign key, and persistence across restarts against live PostgreSQL.
3. **Authenticated Backup Encryption & Hardening (GAP-04, GAP-05, GAP-06):** Upgrade `BackupManager` to support authenticated encryption (AEAD AES-256-GCM / ChaCha20-Poly1305), key versioning, path traversal sanitization, and coordinated consistency barriers.
4. **Full Recovery Drill (GAP-07, GAP-15):** Execute complete end-to-end restoration and service restart.
5. **Systemd Verification (GAP-09):** Execute `systemd-analyze verify` in WSL Ubuntu.
6. **Disaggregated Capacity Model (GAP-14):** Revise `DEPLOYMENT_CAPACITY_AND_COST_MODEL.md` with transparent individual parameters.
