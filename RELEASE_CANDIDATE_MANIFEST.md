# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc3 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21J Distributed Restore Convergence & Batch 21K Production Hardening  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  
**Source Commit Revision:** `2ef6bd40800f26b3a45e6982e315ca005f251b63`  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
c6159534e9b4fa218121cd4f89a3c5d1ab51199e751023403276b07fded70f87  spine-api.exe
a4ad1827c9a0c967268bebaccf1ba5342ce633c355816dbc754e96b62091e4c2  spine-coordinator.exe
a7c82c192dcc8af83413a7bc8458cea6d8191cb06b59bbada21abae2587e36b5  spine-storage-node.exe
f8743f24eb42c49d316b23e35b1a952e781a12beee52416ff4c137b57fa8cf3d  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app` with structured connection URL building, percent-encoding of credentials, and elimination of raw environment variable credential interpolations in `docker-compose.prod.yml`. Enforces non-default storage root under `core-forge-v1`.
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
- **Out-of-Band Volume Backup Runbook ($> 15\text{ MiB}$):** For clusters where individual storage nodes exceed the 15 MiB compressed payload ceiling, documented operational runbook [`docs/OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md`](docs/OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md) specifies atomic block/filesystem snapshotting, writer draining, and generation alignment procedures.
