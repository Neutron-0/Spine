# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc2 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21G Deployment Readiness Candidate  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
5e6c91e7931b0ffd4f278d7c6c023ff0c1a50fa8b9cff7090461c2da912fea00  spine-api.exe
fa149656e7f65557fe0a566dd534cde0f6874660accadd15fd56d9a1b71c87de  spine-coordinator.exe
5e875c96bdfc82e924e381ff3852d71a2ecf35d1f3114811db93fcabef265dbd  spine-storage-node.exe
8e094c542d7350be8671f7d5d76303b0ed6e0274aa72161792700bc8d244b788  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app` with structured connection URL building and credential percent-encoding. Participates in global fencing barrier.
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication. Parameterized dynamic role setup via `01-init-roles.sh` with `SELECT format(...) \gexec` `%I`/`%L` quoting and distinct secrets (`POSTGRES_*_PASSWORD_FILE`, `POSTGRES_ADMIN_PASSWORD:?required`).
- **Consensus Coordinator (`spine-coordinator`):** Raft consensus node on `spine-internal`. Guarded by maintenance barrier, persisted monotonic fencing generation (`.coordinator_fencing_generation`), and crash-resilient rollback state.
- **Storage Nodes (`spine-storage-node-1/2/3`):** Chunked BLAKE3 storage nodes on `spine-internal`. Guarded by maintenance barrier, persisted monotonic fencing generation (`.fencing_generation`), and 6-phase distributed cutover with `.old_restore_<id>` retention.
- **Container Hardening:** All services specify `security_opt: [no-new-privileges:true]`, `cap_drop: [ALL]`, `read_only: true`, non-root users, isolated internal bridge networks, and memory-limited tmpfs mounts (`/tmp`).

---

### 3. PostgreSQL Security & Role Matrix
| Role Name | Scope | Privileges |
|---|---|---|
| `spine_admin` | Database Cluster | Superuser / bootstrap administrative user. Container init only. |
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
- **Persistent Monotonic Fencing Tokens:** Fencing tokens are persisted to disk atomically (`.fencing_generation` and `.coordinator_fencing_generation`) and reloaded across process restarts. Generations can never regress.
- **6-Phase Distributed Cutover Protocol:**
  - `Phase 1: PREPARE` — Staged volume restoration across all cluster nodes in isolated directories.
  - `Phase 2: PREPARED` — Target database restoration and SSH host identity unpack/fingerprint verification.
  - `Phase 3 & 4: COMMIT DECISION & PUBLISH` — Atomic directory cutover on coordinator, storage nodes, and API storage while retaining `.old_restore_<id>` and `.old_generation_<id>`.
  - `Phase 5: VERIFY` — Distributed health check, generation check, and integrity validation across all participants. On failure, all participants execute distributed rollback to prior generations.
  - `Phase 6: FINALIZE` — Retained old generation directories are purged only after global verification succeeds.
- **Fail-Closed Startup Recovery:** Incomplete restore sessions, missing generation tokens, or mid-cutover crashes block server startup with a fatal error, preventing split-brain data corruption.
