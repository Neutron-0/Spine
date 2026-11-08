# Spine Release Candidate Manifest (Batch 21C Deployment Candidate)

**Milestone:** Spine Batch 21C — Critical Backup Consistency, Restore Atomicity, Database Roles, and Deployment Remediation<br>
**Baseline Commit:** `cc075d488dc78a506545465eb59a5d2e927d892e`<br>
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
- `docker-compose.prod.yml`: Production Compose override with isolated `spine-internal` network and `spine-ingress-proxy` network, exposing ONLY ports `80`, `443`, and `2222`.
- `deploy/nginx/ingress.conf`: Production Nginx reverse-proxy ingress configuration with TLS 1.2/1.3, HSTS, and HTTP -> HTTPS 301 redirection.

### 1.4. PostgreSQL Migrations & Consistency Barrier
- Directory: `github-alternative/backend/apps/spine-api/migrations/`
  - `0001_core_identity.sql` through `0012_workflow_provenance.sql`.
  - Transaction-level advisory lock (`pg_advisory_xact_lock(0x5350494E454D4947)`).
  - Globally atomic cluster maintenance advisory lock (`0x5350494E_4542414B`) coordinating point-in-time backups across distributed OS processes.
  - Granular privilege model: `spine_migrator` (DDL `CREATE` permissions) and `spine_app` (runtime DML permissions with `REVOKE CREATE ON SCHEMA public`).

---

## 2. Cryptographic and Environmental Specifications

- **Hash Algorithm:** BLAKE3 (Authoritative object store), SHA-256 (Git remote transport, LFS objects, backup manifests).
- **Password Hashing:** Argon2id with random 16-byte salt.
- **Backup Encryption:** `SPNEBAK v2` streaming chunked AES-256-GCM AEAD encryption with PBKDF2-HMAC-SHA256 key derivation ($10,000 \le \text{iters} \le 1,000,000$), 12-byte per-chunk nonces, 29-byte AAD binding, and authenticated final manifest. Memory usage strictly bounded to $O(1)$ (1 MiB RAM). Strict rejection of trailing bytes, full stream decryption verification on rotation against `sink()`, canonical same-path collision rejection, staged archive creation (`.tmp_bak_<uuid>`), and failure-atomic publish.
- **Database Dump/Restore Format:** Version 3.0 streaming JSON Lines (`metadata`, `table_start`, `row`, `table_end`) with `serde_json` arbitrary precision feature ensuring exact numeric precision roundtrip without IEEE-754 precision loss. Bounded line length (16 MiB max). Exact sequence `last_value` and `is_called` state resync.
- **SSH Protocol:** Pure Rust implementation via `russh`, supporting `ed25519` host keys, user public keys, and repository deploy keys.
- **Database Engine:** PostgreSQL 18.6+ with SCRAM-SHA-256 authentication, least-privilege `spine_app` role (`NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS`), local/WSL subnet binding (`172.18.64.0/20`), and transaction-scoped advisory locking.

---

## 3. Verified Deployment Profiles

1. **`core-forge-v1`**: Control plane, web UI, Git remotes, persistent PostgreSQL. Untrusted external CI execution and shared/public runners strictly prohibited at API, scheduler, runner assignment, and executor layers. Authenticated workflow provenance persisted in PostgreSQL.
2. **`development`**: Local developer evaluation profile permitting ephemeral in-memory storage.

---

## 4. Remediation Certification Status

All findings from the Codex deployment audit (F-01 through F-07) have been remediated down to the source code and programmatically verified against real PostgreSQL:
- **F-01**: PostgreSQL session advisory lock (`0x5350494E_4542414B`) providing cross-process atomic maintenance coordination, write permit wiring across all routes, maintenance held across full snapshot and validation, exact sequence resync, bounded lines.
- **F-02**: Staged backup creation, key rotation full stream decryption verification, failure-atomic storage extraction with validation and rollback.
- **F-03**: Explicit `TrustClassification` in executor; fail-closed provenance evaluation.
- **F-04**: Role split between DDL migrator and runtime DML application role.
- **F-05**: Dockerfile modern Rust alignment, ingress network isolation.
- **F-06**: Preflight archive format, keys, and database empty checks before touching disk.
- **F-07**: Migration cancellation safety verified during active DDL and inserts across 5 cycles.
