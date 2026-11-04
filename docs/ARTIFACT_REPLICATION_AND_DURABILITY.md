# ARTIFACT REPLICATION AND DURABILITY

## 1. Durability Policies
In Spine distributed deployments, artifacts adhere to the same durability guarantees established for repository packfiles in Batch 07 and Batch 10:

| Policy | Required Replicas | Failure Tolerance | Typical Deployment |
|---|---|---|---|
| `SingleCopy` | 1 storage node | 0 node failures | Development / Local Workstation |
| `TwoCopies` | 2 storage nodes | 1 node failure | Small Team / Edge Cluster |
| `ThreeCopies` | 3 storage nodes | 2 node failures | Enterprise Production Forge |

---

## 2. Chunk Replication Architecture

When an artifact session finalizes, or when a release/package binary is stored via `DistributedArtifactStore`:
1. **Target Selection**: The storage coordinator queries active nodes healthy in the current cluster epoch.
2. **Parallel Fan-out**: The manifest and its constituent chunks are streamed across the quorum of target storage nodes.
3. **Quorum Acknowledgement**:
   - `SingleCopy`: Write completes when the primary node writes to disk and fsyncs.
   - `TwoCopies`: Write completes when at least 2 nodes acknowledge durable persistence.
   - `ThreeCopies`: Write completes when at least 3 nodes across failure domains acknowledge durable persistence.

```
       +------------------------------------+
       |      DistributedArtifactStore      |
       +------------------------------------+
              │            │            │
       (Node 1: 4 MiB) (Node 2: 4 MiB) (Node 3: 4 MiB)
              ▼            ▼            ▼
         [Node Alpha] [Node Beta]  [Node Gamma]
              │            │            │
              └────────────┼────────────┘
                           ▼
             Quorum Reached: ACK Client
```

---

## 3. Anti-Entropy and Repair
If a storage node experiences hardware degradation or returns from network partition:
- The coordinator compares storage node manifest and chunk catalogs against the cluster state.
- Missing chunks are repaired in the background via node-to-node replication streams without impacting user read/write latency.
- Corrupted chunks (detected via BLAKE3 mismatch during download) trigger immediate repair from healthy replicas.
