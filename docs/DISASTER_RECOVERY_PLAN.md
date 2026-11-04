# Spine Disaster Recovery & Business Continuity Plan

## 1. Objectives & Metrics

- **Recovery Point Objective (RPO):**
  - Production standard: <= 1 hour (via hourly storage snapshots + PostgreSQL WAL archiving).
  - Minimum baseline: <= 24 hours (nightly coordinated backup archives).
- **Recovery Time Objective (RTO):**
  - Production standard: <= 30 minutes from cold standby to fully operational cluster.
  - Verification target: Restoration rehearsal demonstrated in `< 5 minutes` for medium dataset (< 200 GB).

---

## 2. Disaster Scenarios & Mitigation

### Scenario A: Complete Host Failure (Compute Loss)
- **Impact:** Server hardware or cloud VM permanently lost.
- **Procedure:**
  1. Provision fresh node matching architecture specification.
  2. Install Spine binaries or pull container images.
  3. Attach backed-up persistent storage volume or restore from `spine-api backup restore`.
  4. Point DNS records (`FORGE_PUBLIC_ORIGIN`) to the new host IP.
  5. Validate `/healthz` and test `git clone` via HTTP and SSH.

### Scenario B: Database Storage Corruption or Loss
- **Impact:** PostgreSQL data directory corrupted; storage nodes remain intact.
- **Procedure:**
  1. Isolate the cluster by stopping `spine-api` to prevent inconsistent writes.
  2. Restore PostgreSQL database from latest verified dump (`pg_restore` or `snapshot.json`).
  3. Execute `spine-api migrate` to ensure all 11 schema migrations are aligned.
  4. Run repository integrity reconciliation (`spine fsck`) across storage roots to ensure all referenced objects exist.
  5. Restart `spine-api` and verify `/readyz`.

### Scenario C: Storage Node Disk Loss (Data Loss in Zone A)
- **Impact:** Physical drive or volume holding `node-1` repositories fails.
- **Procedure:**
  1. The Spine Coordinator detects heartbeat timeout and marks `node-1` DEAD.
  2. The Coordinator reassigns primary ownership epochs to surviving replicas (`node-2`, `node-3`) in accordance with replication durability policies.
  3. Provision a new disk or node `node-4` and register with coordinator.
  4. The Coordinator triggers background backfill replication from surviving replicas to re-establish required replication factor (R=3).
