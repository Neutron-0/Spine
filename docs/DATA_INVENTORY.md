# Spine Forge — Authoritative Cluster Data Inventory & Persistence Architecture
**Batch 21F Operational Engineering Specification & Canonical Resource Inventory**

## 1. Overview and Boundary Scope

Spine Forge maintains high-availability, durable source control and collaboration infrastructure. Persistent state is partitioned into distinct authoritative persistent resources to guarantee durability, least privilege, zero data loss, and crash-recoverable restoration.

Every persistent mutation across all resources is strictly governed by:
1. **The Global Consistency Barrier & Monotonic Fencing Authority:** Coordinates across all distributed cluster nodes and operating-system processes via PostgreSQL session advisory locking (`SPINE_CLUSTER_MAINTENANCE_LOCK_ID = 0x5350494E_4542414B`) and the monotonic fencing generation counter in `_cluster_maintenance_barrier`.
2. **Strict Multi-Volume Generation Identity:** Deployment-wide generation identity stamped across PostgreSQL (`_restore_generation`), API storage (`.spine_storage_generation`), storage nodes (`.spine_storage_generation`), and coordinator (`coordinator.state`).
3. **Monotonic Primary Ownership Fencing:** Primary storage node ownership epochs and coordinator fencing tokens preventing split-brain writes.

---

## 2. Canonical Persistent Resource Inventory

| Resource Identifier | Service Owner | Docker Volume Name | Container Destination | Physical Storage Root | Every Writer | Backup Participant Implementation | Restore Participant Implementation | Generation Identity | Expected Manifest | Integrity Verification | Authoritative vs Reconstructible |
|---|---|---|---|---|---|---|---|---|---|---|---|
| `postgres_data` | `spine-api` | `postgres_data` | `/var/lib/postgresql/data` | Docker volume `postgres_data` | PostgreSQL 16 engine, `spine-api` (via `spine_migrator` for DDL, `spine_app` for runtime DML) | `spine-api` streaming JSON Lines dumper (`dump_database_to_writer`) | `spine-api` streaming JSON Lines transactional restorer (`restore_database_from_reader`) with transactional `_restore_generation` insertion | `_restore_generation.generation_token` (UUID v4) | `database/snapshot.jsonl` (Header v3.0, table records, footer summary) | Line-by-line JSON validation, row/table count verification against footer, sequence resync validation | **Authoritative** (primary store of metadata, users, permissions, issues, PRs, CI state) |
| `spine_storage` | `spine-api` | `spine_storage` | `/var/lib/spine/storage` | Docker volume `spine_storage` | `spine-api` backend (Git smart HTTP, LFS transfer, artifact store, package uploads) | `BackupManager::create_archive_internal` | `StagedRestoreSession::publish_storage` (whole-generation directory cutover) | `.spine_storage_generation` | `storage/` directory tree in archive | BLAKE3 / SHA-256 file checksums in `manifest.json`, bounds checking, path traversal rejection | **Authoritative** (Git repositories, LFS blobs, artifacts, releases, cargo crates) |
| `storage_1_data` | `spine-storage-node-1` | `storage_1_data` | `/var/lib/spine/node-1` | Docker volume `storage_1_data` (Zone A) | `spine-storage-node-1` (`stage_pack`, `commit_pack`, `store_chunk`, `store_manifest`) | Storage Node RPC `CaptureVolumeSnapshot` | Storage Node RPC `StageVolumeRestore` -> `CommitVolumeRestore` (whole-generation cutover) | `.spine_storage_generation` | `storage_node_1/volume_manifest.json` | SPK1 packfile magic & BLAKE3 checksums, SPIDX index verification, chunk SHA-256 | **Authoritative** (distributed Git pack shards, content-addressed chunks, manifests) |
| `storage_2_data` | `spine-storage-node-2` | `storage_2_data` | `/var/lib/spine/node-2` | Docker volume `storage_2_data` (Zone B) | `spine-storage-node-2` (`stage_pack`, `commit_pack`, `store_chunk`, `store_manifest`) | Storage Node RPC `CaptureVolumeSnapshot` | Storage Node RPC `StageVolumeRestore` -> `CommitVolumeRestore` (whole-generation cutover) | `.spine_storage_generation` | `storage_node_2/volume_manifest.json` | SPK1 packfile magic & BLAKE3 checksums, SPIDX index verification, chunk SHA-256 | **Authoritative** (distributed Git pack shards, content-addressed chunks, manifests) |
| `storage_3_data` | `spine-storage-node-3` | `storage_3_data` | `/var/lib/spine/node-3` | Docker volume `storage_3_data` (Zone C) | `spine-storage-node-3` (`stage_pack`, `commit_pack`, `store_chunk`, `store_manifest`) | Storage Node RPC `CaptureVolumeSnapshot` | Storage Node RPC `StageVolumeRestore` -> `CommitVolumeRestore` (whole-generation cutover) | `.spine_storage_generation` | `storage_node_3/volume_manifest.json` | SPK1 packfile magic & BLAKE3 checksums, SPIDX index verification, chunk SHA-256 | **Authoritative** (distributed Git pack shards, content-addressed chunks, manifests) |
| `coordinator_data` | `spine-coordinator` | `coordinator_data` | `/var/lib/spine` | Docker volume `coordinator_data` | `spine-coordinator` (`FileBackedCoordinatorStore`) | Coordinator RPC `CaptureVolumeSnapshot` | Coordinator RPC `StageVolumeRestore` -> `CommitVolumeRestore` | `coordinator.state` metadata snapshot epoch | `coordinator/coordinator_manifest.json`, `coordinator/coordinator.state` | JSON schema validation, checksum verification, node and placement topology sanity | **Authoritative** (cluster topology, node registry, replica mappings, primary ownership epochs, failover state) |
| `spine_ssh` | `spine-api` | `spine_ssh` | `/var/lib/spine/ssh` | Docker volume `spine_ssh` | `spine-api` SSH key generator (on initial setup) | `BackupManager` SSH identity participant | `StagedRestoreSession` SSH identity participant | Host key fingerprint SHA-256 | `ssh/host_keys.manifest.json`, `ssh/ssh_host_ed25519_key`, `ssh/ssh_host_ed25519_key.pub` | OpenSSH key format validation, public-private keypair match verification | **Authoritative** (prevents host key verification failure and man-in-the-middle warnings on client reconnect) |
| `runner_workspaces` | `spine-runner` | N/A (local temp or container volume) | `/tmp/spine-runner` / workspace root | Local filesystem / tmpfs | `spine-runner` execution processes | Excluded (ephemeral) | Excluded (ephemeral) | Transient job ID | N/A | Ephemeral step exit status and log verification | **Reconstructible / Ephemeral** (ephemeral execution sandbox; persistent job logs/status stored authoritatively in `postgres_data`) |

---

## 3. Global Coordination & Monotonic Fencing Authority

1. **Single Authoritative Barrier:**
   All persistent write paths (API HTTP routes, Git smart HTTP, native Git-over-SSH, distributed 2PC pack commits, chunk stores, coordinator state mutations) MUST acquire an active `WritePermit` before mutating authoritative storage.
2. **Commit-Boundary Fencing Validation:**
   `WritePermit::verify_active` does NOT rely on a simple TCP connection check. It checks both `is_active == false` AND matches `fencing_generation == permit.fencing_generation`. When maintenance is entered, the global fencing generation increments monotonically:
   ```sql
   UPDATE _cluster_maintenance_barrier
   SET is_active = TRUE,
       activated_at_nanos = $1,
       activated_by = 'spine-backup',
       reason = 'Coordinated deployment-wide multi-volume backup',
       fencing_generation = _cluster_maintenance_barrier.fencing_generation + 1
   WHERE id = 1
   RETURNING fencing_generation;
   ```
3. **Rejection at Storage Publication Boundary:**
   Any writer that acquired a permit before maintenance entered will be fenced out at commit time because its generation is stale. Storage nodes and coordinator explicitly reject any operation presented with a stale fencing generation or during active maintenance.

---

## 4. Multi-Volume Backup Participant Protocol Sequence

1. **Phase 1: Enter Maintenance Barrier:**
   The backup orchestrator enters maintenance mode in PostgreSQL, increments the global fencing generation, and holds the exclusive advisory lock `0x5350494E_4542414B`.
2. **Phase 2: Notify Participants & Drain Writers:**
   Orchestrator sends `EnterMaintenanceBarrier { fencing_generation }` RPC to:
   - Coordinator
   - Storage Node 1
   - Storage Node 2
   - Storage Node 3
   Every participant stops admitting new writes and drains in-flight operations.
3. **Phase 3: Snapshot Capture & Manifest Generation:**
   - PostgreSQL: Database snapshot streamed under repeatable read isolation (`snapshot.jsonl`).
   - `spine_storage`: Local files staged with checksums into `storage/`.
   - `spine_ssh`: Host keypair staged into `ssh/`.
   - `spine-coordinator`: Persistent state snapshot captured into `coordinator/`.
   - Storage Nodes 1-3: `CaptureVolumeSnapshot` RPC captures packfiles, chunks, and manifests into snapshot payloads.
4. **Phase 4: Unified Multi-Volume Archive Assembly:**
   Root archive manifest (`manifest.json`) aggregates individual volume manifests, participant identifiers, and the uniform snapshot fencing generation.
5. **Phase 5: Release Maintenance Barrier:**
   Orchestrator sends `ExitMaintenanceBarrier { fencing_generation }` to all participants and releases the PostgreSQL advisory lock.

---

## 5. Crash-Recoverable Restore State Machine & Startup Safety

1. **Durable Journal Phases:**
   The restore state machine tracks transitions through explicit durable phases:
   `Initialized` -> `StagedStorage` -> `StagedDb` -> `DbPublished` -> `StoragePublished` -> `Committed`.
2. **Atomic In-Transaction Database Generation Recording:**
   `_restore_generation` is recorded *inside the exact same database transaction* that restores the tables. It is impossible for database restore to commit without simultaneously committing the generation token.
3. **Whole-Generation Storage Replacement:**
   Storage restoration never overlays files. Instead:
   - Target files are assembled in `.staged_generation_<restore_id>`.
   - Live generation directories are moved to `.old_generation_<restore_id>`.
   - Staged generation is moved into the live root.
   - `.spine_storage_generation` is written durably.
   - `.old_generation_<restore_id>` is removed upon commit.
   Files absent from the backup are guaranteed absent after restore.
4. **Fail-Closed Startup Invariant:**
   On process startup (API, coordinator, storage nodes):
   - Restore state is evaluated before listeners open.
   - Any uncommitted journal, generation mismatch, missing generation token, missing table, or query error **FAILS CLOSED** with a fatal diagnostic. The deployment remains offline until recovery is reconciled.
