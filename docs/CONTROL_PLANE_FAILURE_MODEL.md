# Spine Control Plane Failure Model

## 1. Scope & Operational Baseline

This document defines the formal failure model, fault boundaries, and crash recovery invariants for the Spine Distributed Control Plane (`spine-coordinator`) implemented in Batch 07.

> [!IMPORTANT]
> **Batch 07 Architectural Scope:** The current control plane operates with a single authoritative coordinator backed by durable state storage (`CoordinatorStore`). High-availability multi-coordinator consensus (e.g. Raft consensus) is planned for future batches. The failure model in this document strictly accounts for coordinator process restarts, metadata persistence, network partitions, and storage node failure modes under this single-coordinator baseline.

---

## 2. Failure Scenarios and Recovery Invariants

```
                                      +------------------------+
                                      |  Client Initiates Push |
                                      +-----------+------------+
                                                  |
                                                  v
                                      +------------------------+
                                      |   Phase: Preparing     |
                                      +-----------+------------+
                        Crash Before Commit       |
           +--------------------------------------+--------------------------------------+
           |                                                                             |
           v                                                                             v
+--------------------+                                                        +--------------------+
|  Coordinator Crash |                                                        | Prepared Copies >= |
|  Transaction Lost  |                                                        | Policy Required    |
+----------+---------+                                                        +----------+---------+
           |                                                                             |
           v                                                                             v
+--------------------+                                                        +--------------------+
|  Nodes Staged Pack |                                                        |   Phase: Commit    |
|  Discarded via GC  |                                                        |     Decided        |
+--------------------+                                                        +----------+---------+
                                                                                         |
                                                                        Crash After Commit Decided
                                                                                         |
                                                                                         v
                                                                              +--------------------+
                                                                              | Restart Replays    |
                                                                              | CommitPush to All  |
                                                                              +--------------------+
```

### 2.1 Scenario A: Coordinator Crash Prior to `CommitDecided`
- **Condition:** The coordinator crashes while transaction `tx` is in `Initiated` or `Preparing`.
- **Invariant:** The transaction **never took effect**. No reference or generation was advanced in authoritative cluster metadata.
- **Recovery:** Upon restart, the coordinator does not find a `CommitDecided` record for `tx`. The client receives a connection reset or timeout and retries with a new transaction. Uncommitted files left in `<staging>/<tx>/` on storage nodes are cleaned up by periodic abort sweeping or background storage GC.

### 2.2 Scenario B: Coordinator Crash After `CommitDecided`
- **Condition:** The coordinator durably logged `TransactionPhase::CommitDecided` and incremented the committed generation in `CoordinatorStore`, but crashed before all storage nodes completed their atomic move.
- **Invariant:** The transaction **is committed**. It is strictly non-rollbackable.
- **Recovery:** Upon coordinator startup:
  1. The coordinator scans its store for transactions in `CommitDecided`.
  2. It re-issues `CommitPush` commands to all participating storage nodes.
  3. Once all storage nodes acknowledge the atomic promotion, the transaction is marked `Committed`.
  4. Repository durability and consistency remain intact.

### 2.3 Scenario C: Storage Node Crash During Staging
- **Condition:** A storage node dies or loses network connectivity while uploading pack bytes in `Preparing`.
- **Invariant:** If the remaining healthy, prepared nodes meet the `DurabilityPolicy` (e.g. 2 copies prepared out of 3 requested), the coordinator proceeds to `CommitDecided`.
- **Recovery:** The offline node is omitted from the initial commit. When the node recovers and resumes heartbeats, the coordinator's Anti-Entropy engine detects that the node is behind the committed generation (`node_gen < committed_gen`) and dispatches a repair job.

### 2.4 Scenario D: Network Partition Between Coordinator and Storage Node
- **Condition:** Storage node $S_1$ becomes isolated from the coordinator, while remaining reachable by some clients.
- **Invariant:** Fencing tokens prevent $S_1$ from accepting unauthorized writes.
- **Detection & Action:**
  1. The coordinator misses heartbeats from $S_1$ beyond `suspect_threshold_secs` (status $\rightarrow$ `Suspect`) and `offline_threshold_secs` (status $\rightarrow$ `Offline`).
  2. If $S_1$ was primary for any repository, the coordinator increments the repository's `OwnershipEpoch` ($E \rightarrow E + 1$) and promotes an eligible replica.
  3. If $S_1$ subsequently receives a write with stale epoch $E$, its local backend rejects it with `FencingError::StaleEpoch`.

---

## 3. Crash Recovery Matrix

| System Phase | Crash Target | Outcome | Recovery Mechanism |
| :--- | :--- | :--- | :--- |
| **Preparing** | Coordinator | Aborted | Abandoned staging purged by storage node timer |
| **Preparing** | Replica Node | Proceed or Abort | If surviving prepared nodes $\ge$ policy requirement, commit; else abort |
| **CommitDecided**| Coordinator | Committed | Coordinator replay loop re-issues `CommitPush` upon restart |
| **Committed** | Primary Node | Failover Triggered | Health monitor detects offline primary $\rightarrow$ promotes highest-gen replica with epoch bump |
| **Anti-Entropy** | Repair Target | Resumed | Incomplete pack transfers discarded; repair scheduler retries upon next health check |
