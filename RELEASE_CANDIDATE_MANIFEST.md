# SPINE — RELEASE CANDIDATE MANIFEST
## Release Candidate v0.1.0-rc2 — Production Packaging & Artifact Inventory

**Target Milestone:** Batch 21D Deployment Readiness Candidate  
**Target Profile:** `core-forge-v1` (Mandatory PostgreSQL, TLS Ingress, Strict Role Separation)  

---

### 1. Release Binaries & SHA-256 Checksums
Artifacts compiled and verified in `target/release-candidate/`:

```
0ac5d59562aa8ad5228d1957d71ceced55b576e435a70a9fe057e8604b92baa8  spine-api.exe
4393fb17c6771c85ef20d63a2232fa65aab181639a0bb2e5fe37067fa612b95d  spine-coordinator.exe
3d72e5ba1f476d8e5544255c8efc5fc6692814c28998fbaa803605ea9f6e7b34  spine-storage-node.exe
519a6c2ea983bb799cc6edc324dc472d200feb82667cec80c66192cbd24b2c64  spine.exe
```

---

### 2. Service Architecture & Security Topology
- **Edge Reverse Proxy (`ingress`):** Hardened NGINX edge gateway; mandatory HTTP 80 -> HTTPS 443 redirection, TLS 1.2/1.3 only, HSTS headers.
- **Control Plane API (`spine-api`):** Unprivileged user (UID 10001). Connects to PostgreSQL using least-privilege DML role `spine_app`.
- **Database Engine (`postgres`):** Bound to internal private network `spine-internal`. No host port publication.
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
- **Cluster Barrier:** PostgreSQL session advisory lock `0x5350494E_4542414B` held exclusively on dedicated non-pooled connection during snapshot capture.
- **State Machine Restore:** Two-phase staged restore with durable `.spine_restore_journal.json`. Storage cutover occurs only after successful database restore.
- **Memory Bounds:** Manifest capped at 16 MiB, archive file limit 1,000,000, path length 4096 bytes, legacy v1 archive memory capped at 64 MiB.
- **Crash Recovery:** Automatic cleanup of interrupted staging on startup.
