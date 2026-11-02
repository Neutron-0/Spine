# Spine Storage Node Design

## 1. Overview & Objectives

The Spine Storage Node daemon (`apps/spine-storage-node`) is the data plane workhorse of the Spine distributed cluster. It is responsible for:
- Providing persistent, verifiable disk storage for packfiles, indexes, and repository references.
- Participating in two-phase replication transactions orchestrated by the coordinator.
- Enforcing monotonically increasing Ownership Epochs to fence stale primaries.
- Autonomous health and capacity accounting with periodic heartbeating.
- Serving anti-entropy delta requests and pack repair data streams.

---

## 2. Component Architecture

```
+-------------------------------------------------------------+
|                     StorageNodeServer                       |
|  - TCP Listener (0.0.0.0:Port)                              |
|  - Frame-based Length-Delimited JSON Protocol Parser        |
|  - Background Heartbeat Emitter                             |
+-------------------------------------------------------------+
                              |
                              v
+-------------------------------------------------------------+
|                     StorageBackend Trait                    |
+-------------------------------------------------------------+
                              |
                              v
+-------------------------------------------------------------+
|                  FileSystemStorageBackend                   |
|  - Physical Directory Layout Management                     |
|  - Atomic Staging -> Commit Promotion                       |
|  - BLAKE3 Verification & Capacity Accounting                |
|  - Local Fencing & Generation Registers                     |
+-------------------------------------------------------------+
```

---

## 3. Storage Hierarchy & On-Disk Layout

Each storage node is assigned an isolated physical root directory via `--storage-root <PATH>`. Repositories are sharded cleanly by `repo_id`:

```
<storage_root>/
└── <repo_id>/
    ├── epoch             # Plaintext ASCII integer (e.g. "4")
    ├── generation        # Plaintext ASCII integer (e.g. "12")
    ├── objects/
    │   └── pack/
    │       ├── <pack_id>.spk        # Immutable pack binary
    │       └── <pack_id>.spk.idx    # Pack index
    ├── staging/
    │   └── <tx_id>/
    │       ├── <pack_id>.spk        # Pre-commit staged pack
    │       └── <pack_id>.spk.idx    # Pre-commit staged index
    └── tombstones/
        └── <tombstone_id>.json      # GC quarantine markers
```

### 3.1 Two-Phase Staging & Promotion Safety
- **Stage Phase:** The incoming pack data and index bytes are written to `<staging>/<tx_id>/<pack_id>.spk` and `<staging>/<tx_id>/<pack_id>.spk.idx`. The storage node computes the BLAKE3 hash of the staged `.spk` and compares it to the expected checksum. If mismatched, the stage operation returns an error immediately.
- **Commit Phase:** When the coordinator issues `CommitPush`, the storage node validates that the incoming epoch matches the repository's epoch, moves the files from `<staging>/<tx_id>/` to `<objects>/pack/` via atomic rename (`fs::rename`), updates `<repo_id>/generation` to the new generation, and purges the staging directory.
- **Abort Phase:** If the transaction fails or times out, `AbortPush` permanently deletes `<staging>/<tx_id>/`.

---

## 4. Fencing & Stale Primary Prevention

Every write operation requires an `epoch` parameter. 

1. When a storage node starts up or receives an epoch bump from the coordinator (e.g. during failover or lease renewal), it updates `<repo_id>/epoch`.
2. When a `StagePack` or `CommitPack` request arrives, the backend reads `<repo_id>/epoch`.
3. If the incoming request has `request_epoch < local_epoch`, the request is rejected with `FencingError::StaleEpoch`:
   $$\text{Reject if } E_{\text{req}} < E_{\text{local}}$$
4. This ensures that an isolated or partitioned primary that wakes up after a network split cannot append data or corrupt repository state.

---

## 5. Capacity Accounting & Autonomous Heartbeats

The storage node maintains real-time capacity statistics without external OS dependencies:
- **Zero-Dependency Directory Sizing:** An in-place recursive scanner (`compute_dir_size`) walks the local repository trees to calculate exact used bytes.
- **Heartbeat Payload:**
  - `node_id`: Node identifier.
  - `timestamp_nanos`: System monotonic time.
  - `node_generation`: Incremented on config/state change.
  - `active_tasks`: Current concurrent RPC operations.
  - `capacity`: `{ total_bytes, used_bytes, available_bytes }`.
  - `status`: Node self-reported status (`Healthy`, `Draining`, etc.).
- **Transmission:** The background heartbeat loop connects to the coordinator address (`--coordinator-addr`) every `--heartbeat-interval-secs` (default 2 seconds).
