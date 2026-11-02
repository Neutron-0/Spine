# Spine Distributed Garbage Collection & Tombstone Safety

## 1. The Distributed GC Hazard

In a distributed version-control cluster, executing garbage collection (GC) or repacking poses severe data-loss risks if performed naively:
1. **Concurrent Push Race:** A client or replica may be referencing or preparing a pack that another node's local GC identifies as unreachable and deletes.
2. **Asymmetrical Deletion:** If node $A$ deletes a pack while node $B$ is partitioned or lagging, future anti-entropy syncs may attempt to copy the unreachable pack back to node $A$ or fail irrevocably.

To ensure safety, Spine implements **Coordinated Two-Phase Distributed GC with Tombstones** (`spine_cluster::gc`).

---

## 2. Distributed GC Architecture

```
+--------------------------------------------------------------+
|                     Distributed GC Coordinator               |
+--------------------------------------------------------------+
                               |
                               | 1. Authorize GC & Issue Tombstone
                               v
+--------------------------------------------------------------+
|                      Tombstone Record                        |
|  - tombstone_id: UUID                                        |
|  - repo_id: Repository ID                                    |
|  - pack_id: Packfile slated for removal                      |
|  - authorization_generation: Min generation to prune         |
|  - retention_expires_at_nanos: Mandatory grace period clock  |
|  - status: Pending -> Authorized -> Executed                 |
+--------------------------------------------------------------+
                               |
                               | 2. Replicate Tombstones to All Nodes
                               v
+--------------------------------------------------------------+
|                     Storage Node Pruning                     |
|  - Validate: status == Authorized                            |
|  - Validate: node_generation >= authorization_generation     |
|  - Validate: now_nanos >= retention_expires_at_nanos         |
|  - Physical Deletion: Only when ALL conditions hold          |
+--------------------------------------------------------------+
```

---

## 3. Tombstone Lifecycle & Safety Invariants

### 3.1 Mathematical Deletion Predicate
A pack file $p$ associated with tombstone $T$ is eligible for physical deletion on a storage node if and only if:

$$\text{CanDelete}(p, T) \iff \begin{cases}
T.\text{status} = \text{Authorized} \\
G_{\text{node}} \ge T.\text{authorization\_generation} \\
t_{\text{current}} \ge T.\text{retention\_expires\_at\_nanos}
\end{cases}$$

### 3.2 Tombstone States
- **`Pending`**: Pack identified as unreachable during a repacking cycle, but transaction or reference lock grace period has not expired.
- **`Authorized`**: Coordinator has confirmed that no active transaction references the pack across the cluster and stamped the current committed generation.
- **`Executed`**: Physical deletion confirmed across all replica nodes; tombstone retained in audit history.

### 3.3 Safety Invariants
1. **No Uncoordinated Deletion:** Storage nodes are prohibited from deleting packfiles autonomously during local operations. All pack deletions must reference a valid tombstone issued by the coordinator.
2. **Generation Monotonicity:** If a new commit advances the repository generation while a tombstone is pending, the tombstone cannot authorize deletion if the pack is reachable in the new generation.
3. **Grace Period Enforced:** Even with zero active references, the default 24-hour retention period prevents race conditions with detached worktrees or stale client leases.
