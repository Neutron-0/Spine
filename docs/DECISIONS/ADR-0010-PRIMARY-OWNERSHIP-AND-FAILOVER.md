# ADR-0010: Primary Ownership, Fencing Tokens and Controlled Failover

## Status
Accepted

## Context
When a primary storage node fails or becomes partitioned from the network, promoting a replica without strictly fencing the old primary leads to split-brain updates where two nodes independently accept conflicting writes. Furthermore, promoting a replica that has fallen behind would silently lose acknowledged commits.

Spine requires monotonic ownership epochs (fencing tokens) and a strict failover eligibility verification pipeline.

## Decision

1. **Ownership Epochs (Fencing Tokens)**:
   - Every repository possesses a monotonically increasing `ownership_epoch` (u64) managed by the coordinator.
   - When a primary node is assigned or promoted, it receives the latest epoch token.
   - Every mutating request (push, repack, ref update) directed to a storage node must include the valid `(repo_id, ownership_epoch)`.
   - Storage nodes enforce that `request.epoch == current_node_epoch`. Any operation carrying an obsolete epoch is immediately rejected with `FencedEpochError`.
   - A recovered former primary is forbidden from accepting writes until it is explicitly re-enrolled or assigned a new valid epoch by the coordinator.

2. **Node Health State Machine**:
   - Nodes transition through strictly validated states:
     `JOINING` ➔ `HEALTHY` ➔ `SUSPECT` ➔ `OFFLINE` ➔ `QUARANTINED` ➔ `DRAINING` ➔ `RETIRED`.
   - Heartbeats include generation timestamps, capacity, and active error counters.
   - A single missed heartbeat transitions a node to `SUSPECT`. Persistent absence beyond the configurable timeout window marks it `OFFLINE`.

3. **Controlled Failover Eligibility**:
   - Automatic or manual promotion requires a candidate replica to satisfy all of the following criteria:
     1. Node state must be `HEALTHY`.
     2. Node must possess the latest committed repository generation.
     3. Node must have verified 100% object closure and zero missing packfiles.
     4. Node must reside in a permitted failure domain according to placement policy.
   - If no replica satisfies all eligibility requirements, the repository is flagged as `RECOVERY_REQUIRED` to prevent silent data loss.

4. **Promotion Sequence**:
   1. Coordinator confirms primary failure and halts pending write transactions.
   2. Candidate replica eligibility is verified against durable metadata.
   3. Ownership epoch is incremented (`epoch = epoch + 1`).
   4. Coordinator assigns the new primary with the incremented epoch.
   5. Remaining replicas are notified of the new primary and new epoch.
   6. Writes resume once the promotion decision is durably logged.

## Consequences
- Guaranteed split-brain immunity during network partitions.
- Zero data loss during failover: stale replicas are rejected from promotion.
