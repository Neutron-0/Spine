# Spine Release Candidate Manifest (Batch 21K Deployment Candidate)

**Milestone:** Spine Batch 21K — Production Global Restore Recovery, PostgreSQL Staging, Journal Durability, and Backup Confidentiality<br>
**Baseline Commit:** `fa7bd142d4f42d8d86bb04d3c713067b944c77a9`<br>
**Target Release:** Deployment Candidate for Thirteenth Independent Codex Audit<br>
**Target Platform:** Linux x86_64, Windows x86_64  

---

## 1. Release Artifacts & Components

### 1.1. Core Daemon Binaries (Rust Native)
- **`spine-api`**: Primary Forge HTTP API server, Web UI host, Git Smart HTTP endpoint, Git LFS HTTP endpoint, SSH Git server, PostgreSQL database migration manager, streaming `SPNEBAK v2` backup & disaster recovery CLI, and deployment profile enforcement engine (`core-forge-v1`).
- **`spine-coordinator`**: Raft-based cluster state manager, distributed lease coordinator, and node health arbiter.
- **`spine-storage-node`**: Object store engine hosting native `.spine` repositories, BLAKE3 `.spack` packfiles, and artifact stores.
- **`spine`**: Native CLI version-control tool for developers (commits, branches, snapshots, diffs, remotes).

#### Release Binary Hashes (`target/release-candidate/SHA256SUMS`):
```text
b3046c7b4bfd03c731ae7d9ef2771bf7fdd0fb0d4a18c5819025188ee88a5c9f  spine-api.exe
09f06490a69c2211709dc26ae787cd808d3255d14f0c2627e05589da97fa5ea9  spine-coordinator.exe
1c15ea3b4bfe99510ae27f575d1936cf01897379b5c9b315513d74a39597b2b1  spine-storage-node.exe
db092b762833b7c25c75035b055869ccbad1afec14c6c288f6bf8e269414b7c3  spine.exe
```

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

---

## 5. Batch 21I Tenth Audit Remediation Status

All findings from the Tenth Independent Codex Audit (SPINE-21H-01 through SPINE-21H-05) have been addressed and verified:
- **SPINE-21H-01 (P0)**: Complete participant payload contract. All 7 participants (database, coordinator, storage_1_data, storage_2_data, storage_3_data, spine_storage, ssh_identity) stream non-empty payload archives (`.tar.gz`). Bypasses eliminated; strict byte length and SHA-256 verification byte-for-byte upon extraction.
- **SPINE-21H-02 (P0)**: Durable, actionable restore recovery. Restore journal stored in dedicated control directory (`.spine_restore_control/restore_journal.json`), protected with `sync_all()` and atomic replace. Forward recovery on restart advances from `StoragePublished` to `Committed`, purging old generations and staging folders. Incomplete restore states fail closed with actionable diagnostics. Coordinator staged and previous committed states persisted across restarts. SSH identity staged publication after DB and storage cutovers, with rollback backup preserved in `.old_ssh_<id>`.
- **SPINE-21H-03 (P1)**: Coordinator `record_heartbeat` acquires writer guard; rejected during maintenance. Generation monotonicity enforced. Existing participants with missing fencing markers fail closed immediately (`bail!`). Safe atomic replace with `sync_all()`.
- **SPINE-21H-04 (P1)**: Production readiness endpoint `/api/v1/health/ready` checking DB, storage root, coordinator, and all storage nodes with 2s bounded timeouts. Requires >=3 storage nodes under `CoreForge`. Healthchecks configured in `docker-compose.prod.yml` with service dependency wiring.
- **SPINE-21H-05 (P1)**: Fixed SQL identifier vs literal quoting in `01-init-roles.sh`. Canonical database name contract detecting conflicts between `POSTGRES_DB` and `SPINE_POSTGRES_DB`. Immediate fail-fast termination on unreadable or empty password files.

---

## 6. Batch 21K Twelfth Audit Master Remediation Status

All findings from the Twelfth Independent Codex Audit have been systematically resolved, end-to-end integrated, and verified:
- **P0-A (Participant Acknowledgement Integration)**: `record_participant_ack` connected directly to production restore orchestration (`main.rs`) across all 7 participants (`postgres_data`, `spine_storage`, `storage_1_data`, `storage_2_data`, `storage_3_data`, `coordinator_data`, `spine_ssh`). RPC interfaces updated from generic boolean responses to typed `ParticipantAcknowledgement`. Journal strictly blocks `CommitDecision` if any participant is missing or unverified.
- **P0-B (Distributed Startup Recovery & Fencing)**: Startup recovery reconciles all 7 participants across local storage, database, coordinator, and storage nodes. Node fencing generation monotonicity and write-rejection barriers prevent writes during unaligned generations. Incomplete restores fail closed immediately.
- **P0-C (PostgreSQL Isolated Staging & Atomic Cutover)**: Completely eliminated direct restoration into the active production database pool prior to commit decision. Restores are staged in an isolated database `<db>_staged_<restore_id>` (or schema) and validated for completeness and generation token before `CommitDecision`. Atomic cutover (`ALTER DATABASE RENAME`) safely preserves the previous generation until finalization.
- **P0-D (Global Journal Durability)**: Hardened `safe_atomic_replace` with readback validation, parent directory `sync_all()`, and temporary backups so `.last_valid` journal decisions are never destroyed during replace.
- **P1 (Eliminated Plaintext Backup Staging)**: Removed intermediate unencrypted `staging.tar.gz`. Backups stream directly from compression (`GzEncoder`) into `SPNEBAK v2` authenticated ciphertext (`EncryptingWriter`), enforcing zero plaintext on disk for SSH keys and sensitive data.
- **P1 (Readiness & Consistency Barrier Write Gate)**: `/api/v1/health/ready` strictly checks responding node IDs against expected IDs (rejecting `None`) and enforces cluster generation alignment. Consistency barrier write permit acquisition inspects restore journals and rejects writes with `ServiceUnavailable` during active restores or generation divergence.
- **P1 (Regression Defect Resolution)**: Fixed recursive directory migration via `safe_move_path` ensuring absent files/directories in newer generations do not survive cutovers, resolving Windows OS error 650. Passed all tests in `git-alternative` and `github-alternative` 100%.
