# Spine Replication Protocol

## 1. Protocol Architecture & Framing

The Spine Replication Protocol governs communication between the Coordinator and Storage Nodes, as well as peer-to-peer data replication among Storage Nodes.

To maintain minimal overhead, maximum auditability, and independence from heavy third-party RPC frameworks, the transport is built upon a standard TCP connection utilizing length-delimited JSON message frames:

```
+---------------------------+----------------------------------------------+
| Field                     | Size / Format                                |
+---------------------------+----------------------------------------------+
| Frame Length (Payload)    | 4 bytes, Big-Endian Unsigned Integer (`u32`) |
| Payload                   | UTF-8 Encoded JSON Serialization             |
+---------------------------+----------------------------------------------+
```

Maximum single-frame payload size is bounded to 64 MiB by default to prevent memory exhaustion attacks. Payloads exceeding this threshold are chunked or streamed.

---

## 2. Message Types & Schema

All messages serialize into the `ClusterRpcRequest` and `ClusterRpcResponse` enums defined in `spine_cluster::protocol`:

```json
{
  "PreparePush": {
    "tx_id": "tx-8f20a0b1",
    "repo_id": "repo-alpha",
    "epoch": 3,
    "pack_id": "pack-001",
    "pack_checksum": "a3f59e...",
    "pack_data": [ ... ],
    "index_data": [ ... ]
  }
}
```

### 2.1 Request Schema Catalog

1. **`PreparePush`**:
   - Transmitted by Coordinator or Primary Node to staging replicas.
   - Parameters: `tx_id`, `repo_id`, `epoch`, `pack_id`, `pack_checksum` (BLAKE3), `pack_data`, `index_data`.
   - Behavior: Stages files into `<staging>/<tx_id>/`, computes BLAKE3 checksum, validates against `pack_checksum`.
2. **`CommitPush`**:
   - Transmitted by Coordinator when durability policy is met.
   - Parameters: `tx_id`, `repo_id`, `epoch`, `new_generation`.
   - Behavior: Atomically moves staged files into `<objects>/pack/`, advances `<repo_id>/generation` to `new_generation`, cleans staging directory.
3. **`AbortPush`**:
   - Transmitted on transaction cancellation or timeout.
   - Parameters: `tx_id`, `repo_id`.
   - Behavior: Purges `<staging>/<tx_id>/`.
4. **`ReplicatePack`**:
   - Transmitted from Primary to Replica during asynchronous catch-up.
   - Parameters: `repo_id`, `epoch`, `pack_id`, `pack_checksum`, `pack_data`, `index_data`.
5. **`InspectRepository`**:
   - Coordinator querying node state.
   - Parameters: `repo_id`.
   - Response: `RepositoryInspection { repo_id, epoch, generation, pack_count, total_bytes, pack_ids }`.
6. **`RepairPack`**:
   - Anti-Entropy reconciliation request asking a source node for a missing pack stream.
   - Parameters: `repo_id`, `pack_id`.
   - Response: Binary pack stream or error if not found.
7. **`Heartbeat`**:
   - Storage Node periodic ping to Coordinator.
   - Parameters: `NodeHeartbeat` containing node stats, capacity, and active tasks.

### 2.2 Response Schema Catalog

- **`Success`**: Empty acknowledgment with zero error.
- **`InspectionResult(RepositoryInspection)`**: Detailed repository state.
- **`PackData { pack_id, pack_data, index_data }`**: Full pack payload returned during repair.
- **`Error(String)`**: Rejection code (e.g. `FencingError::StaleEpoch`, `ChecksumMismatch`, `CapacityExceeded`).

---

## 3. Anti-Entropy Protocol Flow

When the Coordinator or Replica initiates reconciliation:

```
  Node B (Replica)                 Node A (Primary / Source)
         |                                     |
         | --- 1. InspectRepository(repo) ---> |
         | <-- 2. Pack Catalog [P1, P2, P3] -- |
         |                                     |
         | Local Catalog: [P1]                 |
         | Missing Diff:  [P2, P3]             |
         |                                     |
         | --- 3. RepairPack(repo, P2) ------> |
         | <-- 4. PackData(P2 bytes) --------- |
         | [Verify BLAKE3, Store P2]           |
         |                                     |
         | --- 5. RepairPack(repo, P3) ------> |
         | <-- 6. PackData(P3 bytes) --------- |
         | [Verify BLAKE3, Store P3]           |
         |                                     |
         v                                     v
```
