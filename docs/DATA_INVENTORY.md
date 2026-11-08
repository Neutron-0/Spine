# Spine Forge — Authoritative Cluster Data Inventory & Persistence Architecture
**Batch 21E Operational Engineering Specification**

## 1. Overview and Boundary Scope

Spine Forge maintains high-availability, durable source control and collaboration infrastructure. Persistent state is partitioned into five distinct storage domains to guarantee durability, least privilege, zero data loss, and crash-recoverable restoration.

Every persistent mutation across all domains is governed by:
1. **The Global Consistency Barrier:** Coordinates across arbitrary operating-system processes via PostgreSQL session advisory locking (`0x5350494E_4542414B`) and `_cluster_maintenance_barrier`.
2. **Strict Generation Identity:** Atomic cross-resource pairing of `_restore_generation` in the database and `.spine_storage_generation` in the storage root.
3. **Monotonic Ownership Fencing:** Primary storage node ownership epochs preventing stale brain split.

---

## 2. Authoritative Data Domains

### Domain 1: API Relational Database (`postgres_data`)
- **Volume / Mount:** `postgres_data` mounted at `/var/lib/postgresql/data`
- **Engine:** PostgreSQL 16 (Alpine)
- **Role Isolation:**
  - `spine_admin`: Superuser bootstrap identity, never exposed to application workloads.
  - `spine_migrator`: DDL schema owner, executes idempotent migration suites.
  - `spine_app`: Least-privilege runtime identity (DML only: SELECT, INSERT, UPDATE, DELETE). `CREATE ON SCHEMA public` is revoked.
- **Persistent Tables:**
  - **Identity & Organization:** `users`, `sessions`, `organizations`, `teams`, `organization_members`, `team_members`
  - **Repository & Access:** `repositories`, `repository_permissions`, `branch_protections`, `deploy_keys`
  - **Collaboration:** `issues`, `issue_comments`, `pull_requests`, `pull_request_reviews`, `discussions`, `discussion_comments`
  - **Releases & Packages:** `releases`, `release_assets`, `packages`, `package_versions`, `cargo_crates`, `cargo_crate_versions`
  - **CI & Automation:** `workflow_runs`, `workflow_jobs`, `workflow_steps`, `workflow_log_chunks`, `workflow_secrets`, `runner_registrations`
  - **Webhooks:** `webhooks`, `webhook_deliveries`
  - **Coordination & Recovery:** `_cluster_maintenance_barrier`, `_restore_generation`
- **Backup & Snapshot:** Row-by-row JSON Lines streaming (`database/snapshot.jsonl`) under `REPEATABLE READ READ ONLY` snapshot isolation. Sequences synchronized via catalog tables (`pg_class`, `pg_depend`, `pg_attribute`).

---

### Domain 2: Application Storage Root (`spine_storage`)
- **Volume / Mount:** `spine_storage` mounted at `/var/lib/spine/storage`
- **Component:** `spine-api`
- **Contents:**
  - `repositories/<native_repo_id>/`: Bare VCS repositories containing native objects, commits, refs, and packfiles.
  - `artifacts/`: Build outputs, release binaries, and packaged distribution assets managed by `spine_artifact::LocalArtifactStore`.
  - `.spine_storage_generation`: Generation token file stamping the cluster restore generation identity.
  - `.spine_restore_journal.json`: Ephemeral crash-recovery journal tracking restore state machine transitions (`Initialized` -> `StagedStorage` -> `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`).

---

### Domain 3: Distributed Storage Node Volumes (`storage_1_data`, `storage_2_data`, `storage_3_data`)
- **Volumes / Mounts:**
  - `storage_1_data` -> `/var/lib/spine/node-1` (Zone A)
  - `storage_2_data` -> `/var/lib/spine/node-2` (Zone B)
  - `storage_3_data` -> `/var/lib/spine/node-3` (Zone C)
- **Component:** `spine-storage-node`
- **Contents:**
  - `repos/<repo_id>/.spine/`:
    - `pack/`: BLAKE3-checksummed packfiles (`<pack_id>.spk`) and binary index files (`<pack_id>.spidx`).
    - `ownership_epoch`: Monotonically increasing u64 primary ownership fencing epoch.
    - `generation`: Monotonically increasing push generation counter.
  - `staging/<tx_id>/`: Two-phase commit staged packfiles and metadata before cutover.
  - `chunks/<prefix>/<chunk_hash>`: Content-addressed BLAKE3 raw blob chunks.
  - `manifests/<prefix>/<manifest_id>.json`: Chunk composition manifests.

---

### Domain 4: Cluster Coordinator State (`coordinator_data`)
- **Volume / Mount:** `coordinator_data` mounted at `/var/lib/spine`
- **Component:** `spine-coordinator`
- **Contents:**
  - `coordinator.state`: Atomic file-backed persistent state tracking:
    - Node registrations, endpoints, capacity, and failure domain topology.
    - Repository placement plans and replica mappings.
    - Monotonic primary ownership epochs and automated failover records.
    - Node enrollment tokens.

---

### Domain 5: SSH Host Identity (`spine_ssh`)
- **Volume / Mount:** `spine_ssh` mounted at `/var/lib/spine/ssh`
- **Component:** `spine-api` (Native SSH Server on port 2222)
- **Contents:**
  - `ssh_host_ed25519_key`: Ed25519 host private key.
  - `ssh_host_ed25519_key.pub`: Ed25519 host public key.
  - Ensures persistent SSH server identity across container recreations and restarts.

---

## 3. Crash Recovery and Startup Invariant Enforcement

1. **State Machine Verification:**
   On startup, `handle_startup_recovery` inspects `.spine_restore_journal.json`.
   - If an interrupted restore is in `Initialized`, `StagedStorage`, or `StagedDb`, the staging directory and journal are safely discarded because no live state was modified.
   - If in `DbPublished`, `StoragePublished`, or `Failed`, the server **FAILS CLOSED** with a fatal error, preventing startup with half-cutover state.

2. **Cross-Resource Generation Alignment:**
   On startup, `handle_startup_recovery` queries `_restore_generation` in the database and reads `.spine_storage_generation` in the storage root.
   - Any generation mismatch or missing generation file **FAILS CLOSED** with an immediate fatal exit.
   - Server readiness probe (`/health/ready`) will never report ready if data generation is mismatched.
