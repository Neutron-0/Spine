# Spine Release Candidate Manifest (Batch 21B Deployment Candidate)

**Milestone:** Spine Batch 21B — Third Independent Audit Findings Remediation & Release Candidate Hardening<br>
**Baseline Commit:** `a3d44568d1e3333a82e0a983970929a37fb3f37d`<br>
**Target Release:** Deployment Candidate for Independent Codex Re-Audit<br>
**Target Platform:** Linux x86_64, Windows x86_64  

---

## 1. Release Artifacts & Components

### 1.1. Core Daemon Binaries (Rust Native)
- **`spine-api`**: Primary Forge HTTP API server, Web UI host, Git Smart HTTP endpoint, Git LFS HTTP endpoint, SSH Git server, PostgreSQL database migration manager, streaming `SPNEBAK v2` backup & disaster recovery CLI, and deployment profile enforcement engine (`core-forge-v1`).
- **`spine-coordinator`**: Raft-based cluster state manager, distributed lease coordinator, and node health arbiter.
- **`spine-storage-node`**: Object store engine hosting native `.spine` repositories, BLAKE3 `.spack` packfiles, and artifact stores.
- **`spine`**: Native CLI version-control tool for developers (commits, branches, snapshots, diffs, remotes).

### 1.2. Frontend Web Assets
- **`github-alternative/frontend/dist/`**: Production-optimized Single Page Application (Vite 6.4.5 + TypeScript + React/Tailwind), compiled cleanly with zero TypeScript errors and 0 vulnerabilities from `npm audit`.

### 1.3. Systemd Service Definitions & Production Compose
- `systemd/spine-api.service`: Hardened systemd unit for API and SSH daemon.
- `systemd/spine-coordinator.service`: Hardened systemd unit for coordinator daemon.
- `systemd/spine-storage-node.service`: Hardened systemd unit for storage node daemon.
- `docker-compose.yml`: Base Compose configuration with zero internal service host port pollution.
- `docker-compose.dev.yml`: Local developer override exposing `3000:3000` and `5173:80`.
- `docker-compose.prod.yml`: Production Compose override with isolated `spine-internal` bridge network, exposing ONLY ports `80`, `443`, and `2222`.
- `deploy/nginx/ingress.conf`: Production Nginx reverse-proxy ingress configuration with TLS 1.2/1.3, HSTS, and HTTP -> HTTPS 301 redirection.

### 1.4. PostgreSQL Migrations & Consistency Barrier
- Directory: `github-alternative/backend/apps/spine-api/migrations/`
  - `0001_core_identity.sql` through `0012_workflow_provenance.sql`.
  - Transaction-level advisory lock (`pg_advisory_xact_lock(0x5350494E454D4947)`).
  - Database-backed cluster maintenance barrier (`_cluster_maintenance_barrier`) coordinating point-in-time backups across distributed nodes.
  - Granular privilege model: `spine_migrator` (DDL `CREATE` permissions) and `spine_app` (runtime DML permissions with `REVOKE CREATE ON SCHEMA public`).

---

## 2. Cryptographic and Environmental Specifications

- **Hash Algorithm:** BLAKE3 (Authoritative object store), SHA-256 (Git remote transport, LFS objects, backup manifests).
- **Password Hashing:** Argon2id with random 16-byte salt.
- **Backup Encryption:** `SPNEBAK v2` streaming chunked AES-256-GCM AEAD encryption with PBKDF2-HMAC-SHA256 key derivation ($10,000 \le \text{iters} \le 1,000,000$), 12-byte per-chunk nonces, 29-byte AAD binding, and authenticated final manifest. Memory usage strictly bounded to $O(1)$ (1 MiB RAM). Strict rejection of trailing bytes, full manifest verification on rotation, canonical same-path collision rejection, and failure-atomic publish via unique temporary staging.
- **Database Dump/Restore Format:** Version 3.0 streaming JSON Lines (`metadata`, `table_start`, `row`, `table_end`) with `serde_json` arbitrary precision feature ensuring exact numeric precision roundtrip without IEEE-754 precision loss.
- **SSH Protocol:** Pure Rust implementation via `russh`, supporting `ed25519` host keys, user public keys, and repository deploy keys.
- **Database Engine:** PostgreSQL 18.6+ with SCRAM-SHA-256 authentication, least-privilege `spine_app` role (`NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS`), local/WSL subnet binding (`172.18.64.0/20`), and transaction-scoped advisory locking.

---

## 3. Verified Deployment Profiles

1. **`core-forge-v1`**: Control plane, web UI, Git remotes, persistent PostgreSQL. Untrusted external CI execution and shared/public runners strictly prohibited at API, scheduler, runner assignment, and executor layers. Authenticated workflow provenance persisted in PostgreSQL.
2. **`development`**: Local developer evaluation profile permitting ephemeral in-memory storage.

---

## 4. Remediation Certification Status

All findings from the third Codex deployment audit (F-01 through F-07) have been remediated down to the source code and programmatically verified against real PostgreSQL:
- **F-01**: Version 3.0 streaming JSON Lines dumper/restorer, exact JSONB arbitrary numeric precision, cluster write maintenance barrier wired to all mutation endpoints.
- **F-02**: Canonical same-path rejection, failure-atomic staging with `.tmp_rot_<uuid>.spnebak`, pre-publish header verification, zero residual disk files on failure.
- **F-03**: Persisted workflow provenance (`source_repo_id`, `is_fork`, `trust_classification`, `provenance_reason`), direct inspection by scheduler and runner lease without unauthenticated re-evaluations.
- **F-04**: Dedicated DDL migration role `spine_migrator` and runtime least-privilege role `spine_app`; verified schema `CREATE` rejection (`42501`) under `spine_app`; narrowed `pg_hba.conf` subnet to WSL-only.
- **F-05**: Host port `3000` segregated from production compose; `validate_public_origin` enforces HTTPS scheme, non-empty host, and rejects userinfo credentials.
- **F-06**: Masked database passwords (`***`) across all logs; strict secret file validation; fail-fast target database connectivity and empty state pre-flight check.
- **F-07**: Verified `test_actual_run_migrations_cancellation_and_recovery` in `tests/advisory_lock_test.rs` demonstrating real `run_migrations` transaction-level advisory lock, mutual exclusion, task cancellation, engine-level lock cleanup, and recovery.
