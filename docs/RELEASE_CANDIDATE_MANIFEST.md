# Spine Release Candidate Manifest (v0.1.0-rc1)

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Release Tag:** `v0.1.0-rc1`  
**Git Commit Target:** Batch 20 Milestone  
**Target Platform:** Linux x86_64, Windows x86_64

---

## 1. Release Artifacts & Components

### 1.1. Core Daemon Binaries (Rust Native)
- **`spine-api`**: Primary Forge HTTP API server, Web UI host, Git Smart HTTP endpoint, Git LFS HTTP endpoint, SSH Git server, database migration manager, and encrypted backup CLI.
- **`spine-coordinator`**: Raft-based cluster state manager, distributed lease coordinator, and node health arbiter.
- **`spine-storage-node`**: Object store engine hosting native `.spine` repositories, BLAKE3 `.spack` packfiles, and artifact stores.
- **`spine`**: Native CLI version-control tool for developers (commits, branches, snapshots, diffs, remotes).

### 1.2. Frontend Web Assets
- **`github-alternative/frontend/dist/`**: Production-optimized Single Page Application (Vite + TypeScript + React/Tailwind), compiled cleanly with zero TypeScript errors.

### 1.3. Systemd Service Definitions
- `systemd/spine-api.service`: Hardened systemd unit for API and SSH daemon.
- `systemd/spine-coordinator.service`: Hardened systemd unit for coordinator daemon.
- `systemd/spine-storage-node.service`: Hardened systemd unit for storage node daemon.

### 1.4. PostgreSQL Migrations
- Directory: `github-alternative/backend/apps/spine-api/migrations/`
  - `0001_core_identity.sql`
  - `0002_organizations_and_teams.sql`
  - `0003_repositories_and_vcs.sql`
  - `0004_issues_and_pull_requests.sql`
  - `0005_notifications_and_events.sql`
  - `0006_artifacts_and_lfs.sql`
  - `0007_releases_and_packages.sql`
  - `0008_workflows_and_runners.sql`
  - `0009_webhooks.sql`
  - `0010_discussions.sql`
  - `0011_security_center.sql`

---

## 2. Cryptographic and Environmental Specifications

- **Hash Algorithm:** BLAKE3 (Authoritative object store), SHA-256 (Git remote transport, LFS objects, backup manifests).
- **Password Hashing:** Argon2id with random 16-byte salt.
- **Backup Encryption:** AES-256-GCM authenticated encryption with PBKDF2-HMAC-SHA256 key derivation (100,000 iterations).
- **SSH Protocol:** Pure Rust implementation via `russh`, supporting `ed25519` host keys and user public keys.
- **Database Engine:** PostgreSQL 18.6+ with SCRAM-SHA-256 authentication and session advisory locking.

---

## 3. Verified Deployment Profiles

1. **`spine-core-forge-v1`**: Control plane, web UI, Git remotes, PostgreSQL persistence. (Untrusted CI execution strictly prohibited).
2. **`spine-ci-runner-v1`**: Isolated containerized build execution communicating strictly via outbound API polling.

---

## 4. Release Certification

All test suites (unit tests, integration tests, PostgreSQL live persistence tests, AEAD backup tests, Git transport tests, systemd unit verification) have executed and passed with 100% success rate.
