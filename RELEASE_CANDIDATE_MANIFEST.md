# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc2 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21E Deployment Readiness Candidate  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
80c0402822e188b21b0a65a2584297035fdb617c8ab002775b7d21c3cc5b78d9  spine-api.exe
4393fb17c6771c85ef20d63a2232fa65aab181639a0bb2e5fe37067fa612b95d  spine-coordinator.exe
3d72e5ba1f476d8e5544255c8efc5fc6692814c28998fbaa803605ea9f6e7b34  spine-storage-node.exe
519a6c2ea983bb799cc6edc324dc472d200feb82667cec80c66192cbd24b2c64  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Frontend (`spine-frontend`):** Attached strictly to `spine-ingress-proxy`. Public network `spine-public` removed, eliminating ingress bypass risks.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app`.
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication. Parameterized dynamic role setup via `01-init-roles.sh`.
- **Consensus Coordinator (`spine-coordinator`):** Raft consensus node on `spine-internal`.
- **Storage Nodes (`spine-storage-node-1/2/3`):** Chunked BLAKE3 storage nodes on `spine-internal`.

---

### 3. PostgreSQL Security & Role Matrix
| Role Name | Scope | Privileges |
|---|---|---|
| `spine_admin` | Database Cluster | Superuser / bootstrap administrative user. Container init only. |
| `spine_migrator` | Database Owner (`spine_forge`) | DDL owner of schema `public`. Runs migrations, alters tables/sequences. |
| `spine_app` | Runtime Application | DML only: `SELECT`, `INSERT`, `UPDATE`, `DELETE` on tables; `USAGE`, `SELECT`, `UPDATE` on sequences. `CREATE` on schema `public` strictly revoked. |

---

### 4. Backup & Disaster Recovery Architecture
- **Format:** `SPNEBAK v2` authenticated streaming encryption (AES-256-GCM, PBKDF2 100,000 iterations).
- **Cluster Barrier:** PostgreSQL session advisory lock `0x5350494E_4542414B` held exclusively on dedicated non-pooled connection during snapshot capture. Runner mutation endpoints participate in barrier.
- **State Machine Restore:** Durable state machine (`Initialized` -> `StagedStorage` -> `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`).
- **Generation Alignment:** Database records generation in `_restore_generation`; storage records `.spine_storage_generation`. Startup fails closed on any generation mismatch.
- **Fail-Closed Startup Recovery:** Incomplete restore sessions or mid-cutover crashes block server startup with a fatal error, preventing split-brain data corruption.
