# Spine Release Candidate Manifest (Batch 21A Deployment Candidate)

**Milestone:** Spine Batch 21A — Second Independent Audit Findings Remediation & Full-Stack Hardening  
**Audited Baseline:** `8caf8639e854a5165478515c633a0e282c757e7b` (`v0.1.0-rc2`)  
**Target Next Release:** `v0.1.0-rc3` (Pending Codex Re-Audit Certification)  
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

### 1.3. Systemd Service Definitions & Production Compose
- `systemd/spine-api.service`: Hardened systemd unit for API and SSH daemon.
- `systemd/spine-coordinator.service`: Hardened systemd unit for coordinator daemon.
- `systemd/spine-storage-node.service`: Hardened systemd unit for storage node daemon.
- `docker-compose.prod.yml`: Production Compose file with isolated `spine-internal` bridge network and unmapped internal ports.
- `deploy/nginx/ingress.conf`: Production Nginx reverse-proxy ingress configuration with TLS 1.2/1.3, HSTS, and HTTP -> HTTPS 301 redirection.

### 1.4. PostgreSQL Migrations & Consistency Barrier
- Directory: `github-alternative/backend/apps/spine-api/migrations/`
  - `0001_core_identity.sql` through `0011_security_center.sql`
  - Fully transactional migration execution with transaction-scoped advisory locking (`pg_advisory_xact_lock(0x5350494E454D4947)`).
  - Database-backed cluster maintenance barrier (`_cluster_maintenance_barrier`) coordinating point-in-time backups across distributed nodes.

---

## 2. Cryptographic and Environmental Specifications

- **Hash Algorithm:** BLAKE3 (Authoritative object store), SHA-256 (Git remote transport, LFS objects, backup manifests).
- **Password Hashing:** Argon2id with random 16-byte salt.
- **Backup Encryption:** `SPNEBAK v2` streaming chunked AES-256-GCM AEAD encryption with PBKDF2-HMAC-SHA256 key derivation ($10,000 \le \text{iters} \le 1,000,000$), 12-byte per-chunk nonces, 29-byte AAD binding, and authenticated final manifest. Memory usage strictly bounded to $O(1)$ (1 MiB RAM). Strict rejection of trailing bytes and full manifest verification on rotation.
- **SSH Protocol:** Pure Rust implementation via `russh`, supporting `ed25519` host keys, user public keys, and repository deploy keys.
- **Database Engine:** PostgreSQL 18.6+ with SCRAM-SHA-256 authentication, least-privilege `spine_app` role (`NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS`), loopback/private network binding, and transaction-scoped advisory locking.

---

## 3. Verified Deployment Profiles

1. **`core-forge-v1`**: Control plane, web UI, Git remotes, persistent PostgreSQL. Untrusted external CI execution and shared/public runners strictly prohibited at API, scheduler, runner assignment, and executor layers.
2. **`development`**: Local developer evaluation profile permitting ephemeral in-memory storage.

---

## 4. Remediation Certification Status

All seven findings from the second Codex deployment audit (F-01 through F-07) have been remediated down to the source code and programmatically verified against real PostgreSQL:
- F-01: Native streaming SQLx row dumper, `REPEATABLE READ READ ONLY` snapshot isolation, catalog-based sequence sync, 500-row batching.
- F-02: Cryptographically bounded SPNEBAK v2 parameters, zero trailing bytes check, full manifest rotation check, bounded $O(1)$ disk staging during archive pack/unpack.
- F-03: Provenance-based trust evaluation; critical ordering defect resolved in runner assignment (evaluates trust BEFORE lease/mutation).
- F-04: WSL2 PostgreSQL service bound strictly to local/private interfaces; least privilege verified.
- F-05: Compose variable syntax fixed; `docker-compose.prod.yml` and `deploy/nginx/ingress.conf` created.
- F-06: File-based secret ingestion; robust relaxed profile parsing; database-backed maintenance barrier.
- F-07: Real concurrent two-connection cancellation safety verified for transaction-level advisory locks.
