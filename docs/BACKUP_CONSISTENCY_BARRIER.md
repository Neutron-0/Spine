# Backup Consistency Barrier Specification

**Module:** `spine_api::backup::consistency`  
**Components:** `ConsistencyBarrier`, `MaintenanceGuard`, `WritePermit`  
**Guarded Pipelines:** Git Smart HTTP `git-receive-pack`, Git LFS upload/locking, Native Git-over-SSH `receive-pack`, Pull Requests, Releases & Assets, Repository Deletion, Branch Protections, Package Registry  
**Milestone:** Batch 21C Remediation (Finding F-01 Globally Atomic Cross-Process Coordination)

---

## 1. Problem Statement

Creating point-in-time backups across distributed storage nodes and PostgreSQL while active repository pushes, ref updates, pull request merges, release asset uploads, or LFS chunk uploads are in-flight risks capturing inconsistent partial states.

Furthermore, in multi-process architectures or multi-worker deployments, barrier coordination must be globally atomic across independent operating-system processes, not merely process-local in memory.

---

## 2. Architecture & Design

The `ConsistencyBarrier` coordinates backup maintenance windows by pausing or draining in-flight writes before snapshots begin:

1. **Globally Atomic Cross-Process PostgreSQL Advisory Locking:**
   - Dedicated Lock ID: `SPINE_CLUSTER_MAINTENANCE_LOCK_ID = 0x5350494E_4542414B` (`"SPNEBAK"` in ASCII).
   - Maintenance Guard acquires an **exclusive session-level advisory lock** via `pg_advisory_lock(...)` on a dedicated database connection.
   - Mutation routes acquire a **shared session-level advisory lock** via `pg_try_advisory_lock_shared(...)` on a dedicated connection as part of `acquire_write_permit()`.
   - Any crash or termination of an operating-system process automatically drops its PostgreSQL session and immediately releases its lock, preventing deadlock or stranded state.

2. **Cluster Database Barrier State (`_cluster_maintenance_barrier`):**
   - The table `_cluster_maintenance_barrier` persists maintenance window metadata (`is_maintenance`, `active_writes`, `entered_at_nanos`).
   - `enter_maintenance` writes an authoritative row to `_cluster_maintenance_barrier` and awaits a draining window for pending in-flight requests.

3. **Complete Mutation Route Coverage:**
   All mutating routes obtain a `WritePermit`:
   - Git Smart HTTP `git-receive-pack`
   - Git LFS batch upload and lock creation/deletion
   - Git SSH `git-receive-pack`
   - Pull request creation, updates, and merges
   - Release creation, modification, deletion, and asset uploads
   - Repository deletion
   - Branch protection rules
   - Package registry publish and yank/unyank

4. **Maintenance Scope Across Full Backup:**
   In the CLI backup workflow, the `_maintenance_guard` is held open continuously across:
   1. PostgreSQL database streaming snapshot dump
   2. Filesystem repository and object store capture
   3. `SPNEBAK v2` authenticated encryption packaging
   4. Archive decryption and validation before release

5. **RAII Deactivation:**
   The `MaintenanceGuard` implements `Drop`. When dropped, it resets `is_maintenance = FALSE` in `_cluster_maintenance_barrier` and releases the exclusive PostgreSQL advisory lock.
