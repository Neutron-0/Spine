# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc3 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21J Distributed Restore Convergence & Batch 21K Production Hardening  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  
**Source Commit Revision:** `daffd17a25e1015c65f65de12812c5c9428bace1`  
**Source Tree Revision:** `812d0a94834d0721e65cfaf7ba9db12190e363b2`  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
7901d0dc6810f81aae86a4fc8c36f34cd85a5ee4b15a677db110795f31a5ab31  spine-api.exe
0b47434eb9e508407c831613d5dec742b014e3e96894e56fbddbfb9dec207ec8  spine-coordinator.exe
1ed8049200a6a2b306ca9d2cffcf2df6b7b0558c9dcd5f4d07d45d18192edd74  spine-storage-node.exe
5a987eb7a1a1c3f6bc017be82560b191ea69bf21dde9751a8c9db43c91faef5f  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers, container healthcheck with live TCP & master process probing (`pidof nginx && nc -z 127.0.0.1 80`).
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks. Gated with HTTP asset healthcheck (`wget -q -O - http://127.0.0.1:80/`).
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app` with structured connection URL building, percent-encoding of credentials, and elimination of raw environment variable credential interpolations in `docker-compose.prod.yml`. Enforces non-default storage root under `core-forge-v1`. Full readiness healthcheck (`/health/ready`).
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication. Configurable role setup supporting `POSTGRES_MIGRATOR_USER` and `POSTGRES_APP_USER` via `upgrade_postgres_roles.sh` with stdin SQL piping and parameterized psql identifiers (no credentials on process command lines).
- **Consensus Coordinator (`spine-coordinator`):** Raft consensus node on `spine-internal` (port `8400`). Guarded by RAII `CoordinatorWriterGuard`, atomic writer tracking, mutation barrier, draining maintenance barrier, persisted monotonic fencing generation (`.coordinator_fencing_generation` with `safe_atomic_replace`), and crash-resilient rollback state.
- **Storage Nodes (`spine-storage-node-1/2/3`):** Chunked BLAKE3 storage nodes on `spine-internal` (port `8401`). Responding with explicit logical node IDs (`node-1`, `node-2`, `node-3`). Guarded by RAII `StorageWriterGuard`, atomic writer tracking, mutation barrier, draining maintenance barrier, persisted monotonic fencing generation (`.fencing_generation` with error propagation and atomic replacement), and distributed volume restore verification.
- **Readiness Probes (`/health/ready` and `/api/v1/health/ready`):** Validates database connectivity, storage root availability, distinct storage node identities (`node-1`, `node-2`, `node-3`), rejection of duplicate endpoints/identities, uniform generation convergence across all participants and coordinator, and verifies that no active restore journal is in progress.

---

### 3. PostgreSQL Security & Role Matrix
| Role Name | Scope | Privileges |
|---|---|---|
| `spine_admin` | Database Cluster | Superuser / bootstrap administrative user. Container init only. Credentials passed strictly over stdin. |
| `spine_migrator` | Database Owner (`spine_forge`) | DDL owner of schema `public`. Runs migrations, alters tables/sequences. |
| `spine_app` | Runtime Application | DML only: `SELECT`, `INSERT`, `UPDATE`, `DELETE` on tables; `USAGE`, `SELECT`, `UPDATE` on sequences. `CREATE` on schema `public` strictly revoked. |

---

### 4. Distributed Multi-Volume Backup & Disaster Recovery Architecture
- **Format:** `SPNEBAK v2` authenticated streaming AEAD encryption (AES-256-GCM, PBKDF2 100,000 iterations). Mandatory encryption for all production backups containing SSH identity or database snapshots. Plaintext production backups fail closed.
- **Mandatory 7-Resource Canonical Production Participant Inventory:**
  1. `postgres_data` (streaming JSON Lines catalog export with generation injection) -> `database/snapshot.jsonl`
  2. `spine_storage` (control plane application repository storage) -> `storage/spine_storage_snapshot.tar.gz`
  3. `storage_1_data` (storage node 1 volume snapshot) -> `storage_nodes/node-1_snapshot.tar.gz`
  4. `storage_2_data` (storage node 2 volume snapshot) -> `storage_nodes/node-2_snapshot.tar.gz`
  5. `storage_3_data` (storage node 3 volume snapshot) -> `storage_nodes/node-3_snapshot.tar.gz`
  6. `coordinator_data` (consensus coordinator Raft snapshot) -> `coordinator/coordinator_snapshot.tar.gz`
  7. `spine_ssh` (authoritative host Ed25519 private/public key identity pair) -> `ssh/ssh_identity_snapshot.tar.gz`
- **Canonical Participant Contract:** Rejects duplicate participant IDs, duplicate archive paths, unexpected path structures, and volume substitutions.
- **Active Writer Tracking & Draining:** Every mutation acquires an RAII writer permit incrementing `active_writers: Arc<AtomicUsize>`. Maintenance barrier requests poll until `active_writers == 0` before establishing fencing tokens, guaranteeing zero in-flight mutations during backups.
- **Persistent Monotonic Fencing Tokens:** Fencing tokens are persisted to disk atomically (`.fencing_generation` and `.coordinator_fencing_generation`) using `safe_atomic_replace` and never advance in-memory state on persistence failure.
- **Global Restore Protocol & Roll-Forward Convergence:**
  - `Phase 1: PREPARE` — Staged volume restoration across all cluster nodes and participants in isolated staging directories; records `ParticipantAcknowledgement`.
  - `Phase 2: PREPARED` — All 7 participants staged and verified against manifest SHA-256 checksums. Journal records `RestorePhase::Prepared`.
  - `Phase 3: GLOBAL COMMIT DECISION` — Durable `CommitDecisionRecord` logged with platform-appropriate atomic durability and `.last_valid` backup before any participant publication. **This decision is irreversible.**
  - `Phase 4: PUBLISH` — Strict ordered publication: Database (`postgres_data`) -> Storage (`spine_storage`) -> Cluster Volumes (`node-1`, `node-2`, `node-3`, `coordinator`) -> Host Identity (`spine_ssh`). Each participant tracks individual publication progress.
  - `Phase 5: VERIFY` — Distributed health check, generation check, and integrity validation across all 7 participants.
  - `Phase 6: GLOBAL FINALIZE` — Retained old generation directories are purged and journal removed only after global verification succeeds across all participants.
- **Deterministic Startup Recovery:** Pre-decision crashes safely abort staging leaving live storage and database untouched. Post-decision crashes roll forward across all participants, ensuring that database, storage, cluster volumes, and SSH converge on the committed target generation without split-brain risk.
- **Out-of-Band Volume Backup Runbook ($> 15\text{ MiB}$):** For clusters where individual storage nodes exceed the 15 MiB compressed payload ceiling, documented operational runbook [`docs/OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md`](docs/OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md) specifies writer draining, multi-volume archive streaming, and strict automated generation verification. Backed by turnkey operational scripts `scripts/backup_oob.sh` and `scripts/restore_oob.sh`.

---

### 5. Reviewer & Operator Environment Verification
When verifying working tree status from containerized sandboxes, external auditor harnesses (e.g. Codex), or non-default user accounts, Git 2.35.2+ requires directory ownership alignment:
```bash
# Automated cross-platform verifiers:
./scripts/verify_workspace_clean.sh      # Linux / WSL / macOS
.\scripts\verify_workspace_clean.ps1    # Windows PowerShell

# Or direct Git inline command bypassing dubious ownership:
git -c safe.directory=* status

# Or configure persistent exception:
git config --global --add safe.directory D:/Vault/Spine
```

---

### 6. Production Runtime Rehearsal & Live Verification Matrix

Live runtime deployment rehearsal performed on Docker Engine Linux containers with full Compose stack (`docker-compose.prod.yml`):

| Component / Subsystem | Rehearsal Target | Measured Runtime Result | Status |
|---|---|---|---|
| **Container Topology** | 8 isolated containers (`spine-edge-ingress`, `spine-frontend`, `spine-api`, `spine-coordinator`, `spine-storage-node-1/2/3`, `spine-forge-postgres`) | All 8 containers reached healthy state (`Up ... (healthy)`), unprivileged UID 10001 volume permissions verified | **VERIFIED** |
| **HTTP -> HTTPS Ingress** | Port 80 redirect | `HTTP/1.1 301 Moved Permanently` -> `Location: https://127.0.0.1/` | **VERIFIED** |
| **HTTPS Edge Ingress** | Port 443 with TLS 1.2/1.3 | `HTTP/1.1 200 OK`, `Strict-Transport-Security: max-age=63072000; includeSubDomains; preload`, UI served | **VERIFIED** |
| **Cluster Readiness Probe** | `GET /health/ready` | `HTTP/1.1 200 OK`: `{"status":"ready","database":true,"storage":true,"coordinator":true,"storage_nodes":true,"storage_nodes_healthy":3,"storage_nodes_required":3,"cluster_consistent":true,"restore_in_progress":false}` | **VERIFIED** |
| **Native SSH Daemon** | Port 2222 Git SSH protocol | Negotiated `SSH-2.0-russh_0.65.1`, `curve25519-sha256`, `chacha20-poly1305@openssh.com`, host key `ssh-ed25519 SHA256:/83dUuG6/BoXR+3U5/ph81vVnMbHt5yYVMVP4k31oBA`, enforced `publickey` authentication | **VERIFIED** |
| **Storage Topology Gate** | `scripts/backup_oob.sh` volume driver check | `Supported storage topology verified: Docker Compose with local volume driver.` | **VERIFIED** |
| **Backup Immutability & Encryption Policy** | `scripts/backup_oob.sh` fail-closed gates | Rejects unencrypted backups; blocks non-production bypass in `core-forge-v1`; enforces `COMPLIANCE` retention mode | **VERIFIED** |
| **Remote S3 Disaster Recovery Rehearsal** | S3 bucket upload & verification with Object Lock | `FATAL: Off-site replication is required (SPINE_BACKUP_REQUIRE_REMOTE=1), but 'aws' CLI is not found on host.`<br>Host lacks `aws` CLI and live cloud S3 bucket with Object Lock Compliance. Marked strictly **UNVERIFIED AT RUNTIME**. | **UNVERIFIED AT RUNTIME (MISSING DEPENDENCY)** |

