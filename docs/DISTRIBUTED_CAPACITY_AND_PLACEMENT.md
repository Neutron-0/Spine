# Spine Distributed Capacity, Placement & Failure Domains

## 1. Failure Domains Hierarchy

To ensure high availability against physical hardware and network partition events, every Spine Storage Node registers its topological location via a hierarchical **Failure Domain** (`spine_cluster::node::FailureDomain`):

```
Region (e.g. "us-east")
  └── Zone (e.g. "zone-a", "zone-b")
        └── Host (e.g. "storage-srv-01", "storage-srv-02")
```

When evaluating replication placements, the scheduler enforces failure domain diversity across all selected replicas.

---

## 2. Placement Scheduling Algorithm

The `PlacementScheduler` (`spine_cluster::placement`) schedules primary and replica assignments according to repository durability policies:

### 2.1 Placement Rules & Constraints

1. **Node Eligibility:** Only nodes in `NodeStatus::Healthy` with sufficient free capacity (`available_bytes >= required_bytes`) are considered.
2. **Failure Domain Diversity:**
   - The primary node $P$ and replica nodes $R_1, R_2, \dots$ are scheduled across **distinct zones** wherever available.
   - If cluster size allows, nodes in different regions or distinct physical hosts within zones are prioritized.
3. **Load Balancing & Capacity Heuristics:**
   Candidates are scored using a composite balancing heuristic:
   $$\text{Score}(N) = w_{\text{cap}} \cdot \frac{\text{AvailableBytes}(N)}{\text{TotalBytes}(N)} - w_{\text{load}} \cdot \text{ActiveTasks}(N)$$
   Nodes with the highest score are selected as candidates.

### 2.2 Mathematical Model of Placement Plan

Given repository $R$, durability policy $D \in \{ \text{SingleCopy}, \text{TwoCopies}, \text{ThreeCopies} \}$, and cluster nodes $\mathcal{N}$:

$$\begin{aligned}
k &= \text{required\_copies}(D) \\
\mathcal{C} &= \{ n \in \mathcal{N} \mid \text{Status}(n) = \text{Healthy} \land \text{CapacityAvailable}(n) > \text{Threshold} \} \\
P &= \arg\max_{n \in \mathcal{C}} \text{Score}(n) \\
\mathcal{R} &= \text{SelectTopK}(\mathcal{C} \setminus \{P\}, k - 1 \mid \text{DistinctZones}) \\
\text{PlacementPlan} &= \{ \text{primary}: P, \; \text{replicas}: \mathcal{R} \}
\end{aligned}$$

---

## 3. Storage Rebalancing Workflow

When new storage nodes are enrolled or existing nodes are drained:
1. `spine storage rebalance --apply` evaluates current repository placements against the optimal placement schedule.
2. For each unbalanced repository:
   - A new placement plan is generated with diverse failure domain allocations.
   - Replicas are populated on the new target nodes via Anti-Entropy replication.
   - Once all new replicas report complete generations and verified pack catalogs, the coordinator updates the authoritative placement metadata.
   - Stale placements on decommissioned nodes are marked for tombstoning.
