# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc2 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21H Final Deployment Architecture Integration & Release Verification  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
c700af4ca653557bc3eef089076f68198a7a6c75e65b8aaf51f16013c7db0579  spine-api.exe
b2aaa1974d53e434debc12a789652827fc12230b1ebf894a2b1256d3dd5c2676  spine-coordinator.exe
694208c62134d4f69d0cace9ae05fd9b95ced3e1964e43a7ab68079e6172bb98  spine-storage-node.exe
519429b3c8f426ae05e86a2004cbfa570590cb131083399d35738d20894d33a6  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app` with structured connection URL building and credential percent-encoding. Participates in global fencing barrier and enforces non-default storage root under `core-forge-v1`.
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication. Parameterized dynamic role setup via `01-init-roles.sh` with stdin SQL piping (no passwords on process command lines) and distinct secrets (`POSTGRES_*_PASSWORD_FILE`, `POSTGRES_ADMIN_PASSWORD:?required`). Supports online credential updates via `scripts/upgrade_postgres_roles.sh`.
- **Consensus Coordinator (`spine-coordinator`):** Raft consensus node on `spine-internal`. Port reconciled to `8400`. Guarded by RAII `CoordinatorWriterGuard`, `active_writers` atomic counter, mutation barrier, draining maintenance barrier, persisted monotonic fencing generation (`.coordinator_fencing_generation`), and crash-resilient rollback state.
- **Storage Nodes (`spine-storage-node-1/2/3`):** Chunked BLAKE3 storage nodes on `spine-internal`. Port reconciled to `8401`. Guarded by RAII `StorageWriterGuard`, `active_writers` atomic counter, mutation barrier, draining maintenance barrier, persisted monotonic fencing generation (`.fencing_generation`), and 6-phase distributed cutover with `.old_restore_<id>` retention.
- **Container Hardening:** All services specify `security_opt: [no-new-privileges:true]`, `cap_drop: [ALL]`, `read_only: true`, non-root users, isolated internal bridge networks, and memory-limited tmpfs mounts (`/tmp`).

---

### 3. PostgreSQL Security & Role Matrix
| Role Name | Scope | Privileges |
|---|---|---|
| `spine_admin` | Database Cluster | Superuser / bootstrap administrative user. Container init only. Credentials passed strictly over stdin. |
| `spine_migrator` | Database Owner (`spine_forge`) | DDL owner of schema `public`. Runs migrations, alters tables/sequences. |
| `spine_app` | Runtime Application | DML only: `SELECT`, `INSERT`, `UPDATE`, `DELETE` on tables; `USAGE`, `SELECT`, `UPDATE` on sequences. `CREATE` on schema `public` strictly revoked. |

---

### 4. Distributed Multi-Volume Backup & Disaster Recovery Architecture
- **Format:** `SPNEBAK v2` authenticated streaming encryption (AES-256-GCM, PBKDF2 100,000 iterations).
- **Mandatory 7-Resource Production Participant Inventory:**
  1. `postgres_data` (streaming JSON Lines catalog export with generation injection)
  2. `spine_storage` (control plane application repository storage)
  3. `storage_1_data` (storage node 1 volume snapshot)
  4. `storage_2_data` (storage node 2 volume snapshot)
  5. `storage_3_data` (storage node 3 volume snapshot)
  6. `coordinator_data` (consensus coordinator Raft snapshot)
  7. `spine_ssh` (authoritative host Ed25519 private/public key identity pair)
- **Active Writer Tracking & Draining:** Every mutation acquires an RAII writer permit incrementing `active_writers: Arc<AtomicUsize>`. Maintenance barrier requests poll until `active_writers == 0` before establishing fencing tokens, guaranteeing zero in-flight mutations during backups.
- **Persistent Monotonic Fencing Tokens:** Fencing tokens are persisted to disk atomically (`.fencing_generation` and `.coordinator_fencing_generation`) and reloaded across process restarts. Generations can never regress.
- **6-Phase Distributed Cutover & Durable Pre-Cutover Decision Ledger:**
  - `Phase 1: PREPARE` — Staged volume restoration across all cluster nodes in isolated directories.
  - `Phase 2: PREPARED` — Target database restoration and SSH host identity unpack/fingerprint verification. Journaled as `RestorePhase::Prepared`.
  - `Phase 3: COMMIT DECISION` — Durable `CommitDecisionRecord` logged to restore journal before any irreversible mutations or live symlink/directory cutovers take place.
  - `Phase 4: PUBLISH` — Atomic directory cutover on coordinator, storage nodes, and API storage while retaining `.old_restore_<id>` and `.old_generation_<id>`.
  - `Phase 5: VERIFY` — Distributed health check, generation check, and integrity validation across all participants. On failure, all participants execute distributed rollback to prior generations.
  - `Phase 6: FINALIZE` — Retained old generation directories are purged only after global verification succeeds.
- **Fail-Closed Startup Recovery:** Interrupted staging sessions are safely cleaned up. Interrupted cutover states (`CommitDecision`, `DbPublished`, `StoragePublished`) fail closed on node startup, halting execution with a fatal diagnostic to prevent split-brain data corruption.
