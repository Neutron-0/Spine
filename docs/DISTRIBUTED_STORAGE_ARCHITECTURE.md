# Spine Distributed Storage Architecture

## 1. Executive Summary & Architecture Philosophy

Spine Distributed Storage Fabric is an original, Rust-native distributed repository management and replication system designed to scale from local developer environments to multi-datacenter cluster topologies. It provides high availability, fault tolerance, verifiable durability, and anti-entropy reconciliation for Spine repositories without reliance on third-party cloud orchestration or proprietary vendor engines.

The architecture strictly separates the **Control Plane** (`spine-coordinator`) from the **Data Plane** (`spine-storage-node`). All data plane write operations are governed by two-phase transactions under explicit Durability Policies, protected by monotonically increasing Ownership Epochs and Fencing Tokens to prevent split-brain anomalies.

```
                                 +--------------------+
                                 |  Spine CLI / API   |
                                 +---------+----------+
                                           |
                    +----------------------+----------------------+
                    | RPC Metadata & Leases                       | RPC Data Stream
                    v                                             v
         +----------------------+                      +----------------------+
         |  Spine Coordinator   |                      | Storage Node (Leader)|
         |  (Control Plane)     |                      |     Epoch: E, Gen: G |
         +----------+-----------+                      +----------+-----------+
                    |                                             |
       Heartbeat &  | Placement Plan                              | 2-Phase Pack
       Health Ping  | Leases & Fencing                            | Replication
                    v                                             v
         +----------------------+                      +----------------------+
         | Storage Node (Follow)|<---------------------+ Storage Node (Follow)|
         | Replica 1            |   Anti-Entropy Diff  | Replica 2            |
         +----------------------+                      +----------------------+
```

---

## 2. Cluster Topology & Core Entities

### 2.1 The Distributed Coordinator (`spine-coordinator`)
The coordinator is the central authority governing cluster topology, node health evaluation, placement scheduling, replication transaction commit decisions, failover promotions, and distributed garbage collection authorization.

- **Authoritative State:** Tracks node registry, repository placements, replica generations, active replication transactions, failure domains, and tombstones.
- **Pluggable Persistence (`CoordinatorStore`):** Designed around an abstract trait with a production-ready PostgreSQL relational mapping (`migrations/20261102000001_cluster_metadata.sql`) and a local atomic file-backed JSON snapshot store (`FileBackedCoordinatorStore`).
- **Zero-Split-Brain Invariant:** A single coordinator instance acts as the epoch and fencing authority. While future batches introduce Raft consensus for multi-coordinator high availability, the Batch 07 control plane is explicitly documented as a single-coordinator model with durable state persistence and crash recovery.

### 2.2 Storage Nodes (`spine-storage-node`)
Storage nodes serve as the data plane daemons. Each node manages physical storage roots on its local filesystem.

- **Local Storage Layout:**
  - `<storage_root>/<repo_id>/epoch`: Monotonic fencing epoch number.
  - `<storage_root>/<repo_id>/generation`: Committed generation counter.
  - `<storage_root>/<repo_id>/objects/pack/`: Immutable `.spk` packfiles and `.spk.idx` indexes.
  - `<storage_root>/<repo_id>/staging/<tx_id>/`: Transaction staging directory for two-phase commits.
  - `<storage_root>/<repo_id>/tombstones/`: Distributed GC quarantine markers.
- **Autonomous Heartbeating:** Periodically measures capacity (`total_bytes`, `used_bytes`, `available_bytes`), active tasks, and node generation, transmitting signed heartbeats to the coordinator.
- **Fencing Enforcement:** Validates that incoming staging and commit requests carry an ownership epoch greater than or equal to the locally persisted epoch.

---

## 3. Node Lifecycle State Machine

Nodes follow strict, deterministic state transitions:

```
    [ Enrolling ]
          |
          v (Token + Ed25519 Verified)
      [ Healthy ] <-------------------+
       |       |                      | (Heartbeats Resumed)
       |       v (Missed Heartbeats)  |
       |    [ Suspect ] --------------+
       |       |
       |       v (Timeout Exceeded)
       |    [ Offline ]
       |       |
       v       v
     [ Draining ] -----------------> [ Quarantined ]
          |                               |
          v                               v
      [ Retired ]                    [ Retired ]
```

1. **Enrolling:** Newly spawned node exchanging cryptographic challenge-response tokens with the coordinator.
2. **Healthy:** Regularly transmitting heartbeats within the healthy window; eligible for both primary and replica placements.
3. **Suspect:** Missed heartbeats beyond `suspect_threshold_secs`; barred from receiving new primary placements; reads may be diverted.
4. **Offline:** Missed heartbeats beyond `offline_threshold_secs`; triggers failover evaluation for any repositories where this node acted as primary.
5. **Draining:** Administratively marked for decommission; active workloads migrated to alternate failure domains.
6. **Quarantined:** Isolated due to checksum corruption or hardware faults.
7. **Retired:** Terminal state; permanently excluded from scheduling.

---

## 4. Durability Policies and Two-Phase Push Commit

Spine enforces deterministic durability invariants before any push transaction is acknowledged to clients:

| Policy | Minimum Verified Copies | Target Replica Set | Failure Tolerance |
| :--- | :---: | :---: | :---: |
| `SingleCopy` | 1 | Primary only | 0 node failures |
| `TwoCopies` | 2 | Primary + 1 Replica | 1 replica failure (with primary intact) |
| `ThreeCopies` | 3 | Primary + 2 Replicas | 1 node failure without durability loss |

### 4.1 Two-Phase Commit Lifecycle
1. **Initiation (`Initiated`):** Client requests push transaction `tx_id` from coordinator. Coordinator generates placement plan and returns fencing token `(epoch, generation)`.
2. **Preparation (`Preparing`):** Primary and replicas stage the new packfile into `<staging>/<tx_id>/`. Each node computes BLAKE3 checksum and reports preparation acknowledgment to coordinator.
3. **Policy Evaluation (`Prepared`):** Coordinator verifies `prepared_nodes.len() >= policy.required_copies()`.
4. **Commit Decision (`CommitDecided`):** Coordinator durably writes the commit record to the metadata store with incremented generation `G + 1`. Once `CommitDecided` is reached, the transaction is non-rollbackable.
5. **Commit Finalization (`Committed`):** Primary and replica storage nodes atomically rename staged packfiles into `<objects>/pack/` and update `generation` counter to `G + 1`.
6. **Client Acknowledgment:** Client receives verified push success.
