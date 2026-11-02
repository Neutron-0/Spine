# Spine Failover, Fencing, and Primary Ownership

## 1. Primary Ownership & The Split-Brain Threat

In a distributed version-control cluster, multiple nodes store copies of a repository. If two storage nodes simultaneously believe they are the authoritative primary ("split-brain"), clients may push divergent histories to each node, resulting in silent data corruption, lost commits, and corrupted branch references.

To eliminate this class of distributed fault, Spine implements:
1. **Authoritative Ownership Epochs:** A strictly monotonic sequence counter associated with every repository:
   $$E_{n+1} > E_n$$
2. **Fencing Tokens:** A cryptographic/metadata tuple `(epoch, generation)` attached to every mutating operation.
3. **Failover Invariants:** A mathematical guarantee that only replicas with the complete committed history can be promoted.

---

## 2. Failover Promotion Invariants

When the coordinator's health evaluator detects that a repository's primary node has transitioned to `Offline` or `Quarantined`, the `FailoverCoordinator` initiates failover.

A candidate replica node $R$ is eligible for promotion if and only if all of the following conditions hold:

$$\text{Eligible}(R) \iff \begin{cases} 
\text{Status}(R) = \text{Healthy} \\
\text{Generation}(R) = \text{CommittedGeneration} \\
\text{IntegrityVerified}(R) = \text{true}
\end{cases}$$

### 2.1 Rejection of Stale Replicas
If a replica lagged behind the primary during previous writes and has $\text{Generation}(R) < \text{CommittedGeneration}$, promotion is strictly disallowed. The failover coordinator will reject the promotion to prevent data regression.

### 2.2 Best Candidate Selection
If multiple replicas satisfy all eligibility criteria, the coordinator selects the optimal candidate by sorting:
1. **Lowest Active Tasks:** $\min(\text{ActiveTasks}(R))$
2. **Highest Available Capacity:** $\max(\text{AvailableBytes}(R))$

---

## 3. Epoch Bumping and Fencing Action

Once candidate $R_{\text{promoted}}$ is selected:
1. The coordinator atomically increments the repository epoch:
   $$E_{\text{new}} = E_{\text{current}} + 1$$
2. The coordinator updates the placement metadata in `CoordinatorStore`:
   - Sets `primary_node_id = R_promoted`
   - Replaces `R_promoted` in `replica_node_ids` with a standby node or records degraded redundancy.
3. The coordinator pushes the epoch update to $R_{\text{promoted}}$.
4. When any request arrives from the old primary $P_{\text{old}}$ carrying $E_{\text{old}}$, the storage nodes and coordinator reject it:
   $$\text{Reject with } \texttt{FencingError::StaleEpoch} \quad \forall \; E_{\text{old}} < E_{\text{new}}$$

---

## 4. Controlled Demotion & Re-integration

When the recovered primary $P_{\text{old}}$ returns online:
1. It connects to the coordinator and presents its status.
2. The coordinator notices $P_{\text{old}}$ is no longer primary and its local epoch $E_{\text{old}} < E_{\text{new}}$.
3. $P_{\text{old}}$ is demoted to a replica.
4. An Anti-Entropy repair job is scheduled to sync any packs written during its downtime.
