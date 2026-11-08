# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc2 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21F Deployment Readiness Candidate  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
d27246f9f66dffcfb3d76bf8b0edf7eb66ee82aa0bec9ed747aeadb616b58261  spine-api.exe
6ca2d304cda19698a79045622909edfb752b6ff7b44e8c84c2405a9df5594681  spine-coordinator.exe
1a590d79c307d288da8ebf87840ee8a43216cc39e8abbeea86556eff04d23ddc  spine-storage-node.exe
e639bdd7d3ef8888bfbf77534d6ad15d45b61c52fa14c8a7551e27268f3ee2a8  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app`. Participates in global fencing barrier.
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication. Parameterized dynamic role setup via `01-init-roles.sh` with distinct secrets (`POSTGRES_*_PASSWORD_FILE`).
- **Consensus Coordinator (`spine-coordinator`):** Raft consensus node on `spine-internal`. Guarded by maintenance barrier and monotonic fencing generation.
- **Storage Nodes (`spine-storage-node-1/2/3`):** Chunked BLAKE3 storage nodes on `spine-internal`. Guarded by maintenance barrier, monotonic fencing generation, and memory-safe whole-generation cutover.
- **Container Hardening:** All services specify `security_opt: [no-new-privileges:true]`, read-only root filesystems where applicable, non-root users, and isolated internal bridge networks.

---

### 3. PostgreSQL Security & Role Matrix
| Role Name | Scope | Privileges |
|---|---|---|
| `spine_admin` | Database Cluster | Superuser / bootstrap administrative user. Container init only. |
| `spine_migrator` | Database Owner (`spine_forge`) | DDL owner of schema `public`. Runs migrations, alters tables/sequences. |
| `spine_app` | Runtime Application | DML only: `SELECT`, `INSERT`, `UPDATE`, `DELETE` on tables; `USAGE`, `SELECT`, `UPDATE` on sequences. `CREATE` on schema `public` strictly revoked. |

---

### 4. Multi-Volume Backup & Disaster Recovery Architecture
- **Format:** `SPNEBAK v2` authenticated streaming encryption (AES-256-GCM, PBKDF2 100,000 iterations).
- **Comprehensive Multi-Volume Scope:** Backs up all 7 persistent deployment volumes: `postgres_data`, `spine_storage`, `storage_1_data`, `storage_2_data`, `storage_3_data`, `coordinator_data`, and `spine_ssh`.
- **Monotonic Fencing Generation Barrier:** PostgreSQL session advisory lock `0x5350494E_4542414B` and `_cluster_maintenance_barrier` issue monotonically increasing generation tokens (`fencing_generation`). Storage nodes and coordinator store validate fencing tokens at commit boundaries.
- **Whole-Generation Directory Replacement:** Restorations stage new versions completely and atomically rename live roots to `.old_generation_<id>` before activating staged trees, preventing absent-file remnants and corruption.
- **Durable State Machine Restore:** `Initialized` -> `StagedStorage` -> `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`.
- **Generation Alignment:** Database records generation in `_restore_generation` inside the active restore transaction prior to commit; storage records `.spine_storage_generation`. Startup fails closed on any generation mismatch.
- **Fail-Closed Startup Recovery:** Incomplete restore sessions, missing generation tokens, or mid-cutover crashes block server startup with a fatal error, preventing split-brain data corruption.
