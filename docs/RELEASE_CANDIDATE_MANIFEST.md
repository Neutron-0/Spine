# Spine Release Candidate Manifest (v0.1.0-rc2)

**Milestone:** Spine Batch 21 — Independent Audit Findings Remediation & Deployment Security Closure  
**Release Tag:** `v0.1.0-rc2`  
**Previous Tag:** `v0.1.0-rc1`  
**Audited Commit:** `f68f3ffc6fe6450424d98df9e8ed1d7a319f3dd0` (Codex Audit 1)  
**Target Platform:** Linux x86_64, Windows x86_64

---

## 1. Release Artifacts & Components

### 1.1. Core Daemon Binaries (Rust Native)
- **`spine-api`**: Primary Forge HTTP API server, Web UI host, Git Smart HTTP endpoint, Git LFS HTTP endpoint, SSH Git server, PostgreSQL database migration manager, streaming `SPNEBAK v2` backup & disaster recovery CLI, and deployment profile enforcement engine (`core-forge-v1`).
- **`spine-coordinator`**: Raft-based cluster state manager, distributed lease coordinator, and node health arbiter.
- **`spine-storage-node`**: Object store engine hosting native `.spine` repositories, BLAKE3 `.spack` packfiles, and artifact stores.
- **`spine`**: Native CLI version-control tool for developers (commits, branches, snapshots, diffs, remotes).

### 1.2. Frontend Web Assets
- **`github-alternative/frontend/dist/`**: Production-optimized Single Page Application (Vite + TypeScript + React/Tailwind), compiled cleanly with zero TypeScript errors. Includes explicit security status badges for Core Forge profile restrictions.

### 1.3. Systemd Service Definitions
- `systemd/spine-api.service`: Hardened systemd unit for API and SSH daemon.
- `systemd/spine-coordinator.service`: Hardened systemd unit for coordinator daemon.
- `systemd/spine-storage-node.service`: Hardened systemd unit for storage node daemon.

### 1.4. PostgreSQL Migrations
- Directory: `github-alternative/backend/apps/spine-api/migrations/`
  - `0001_core_identity.sql` through `0011_security_center.sql`
  - Fully transactional migration execution with transaction-scoped advisory locking (`pg_advisory_xact_lock(0x5350494E454D4947)`).

---

## 2. Cryptographic and Environmental Specifications

- **Hash Algorithm:** BLAKE3 (Authoritative object store), SHA-256 (Git remote transport, LFS objects, backup manifests).
- **Password Hashing:** Argon2id with random 16-byte salt.
- **Backup Encryption:** `SPNEBAK v2` streaming chunked AES-256-GCM AEAD encryption with PBKDF2-HMAC-SHA256 key derivation (100,000 iterations), 12-byte per-chunk nonces, 29-byte AAD binding, and authenticated final manifest. Memory usage strictly bounded to $O(1)$ (1 MiB RAM).
- **SSH Protocol:** Pure Rust implementation via `russh`, supporting `ed25519` host keys, user public keys, and repository deploy keys.
- **Database Engine:** PostgreSQL 18.6+ with SCRAM-SHA-256 authentication, least-privilege `spine_app` role (`NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS`), loopback/private network binding, and transaction-scoped advisory locking.

---

## 3. Verified Deployment Profiles

1. **`core-forge-v1`**: Control plane, web UI, Git remotes, persistent PostgreSQL. Untrusted external CI execution and shared/public runners strictly prohibited at API, scheduler, and executor layers.
2. **`development`**: Local developer evaluation profile permitting ephemeral in-memory storage.

---

## 4. Release Certification

All seven findings from the first Codex deployment audit (F-01 through F-07) have been remediated with verified automated regression tests. Codebase passes `cargo clippy -D warnings` and `cargo fmt --check` with zero warnings or errors across both workspaces.
