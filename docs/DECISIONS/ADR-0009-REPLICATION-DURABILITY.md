# ADR-0009: Replication Durability & Distributed Transaction Protocol

## Status
Accepted

## Context
In a distributed version control platform, acknowledging a client push before data is verified across the required number of replicas risks silent data loss if the primary node crashes immediately after receiving the write. Furthermore, network partitions and partial failures can leave nodes with mismatched repository generations.

Spine requires a deterministic, coordinator-managed 2-phase write transaction protocol and explicit durability policies.

## Decision

1. **Durability Policies**:
   - `SINGLE_COPY`: 1 verified durable copy (the primary node). Suitable only for ephemeral caches or non-critical scratch mirrors.
   - `TWO_COPIES`: Primary plus 1 verified replica (2 total copies).
   - `THREE_COPIES`: Primary plus 2 verified replicas (3 total copies).
   - Push operations **must not** report success to the client until the required number of verified nodes have acknowledged durability.

2. **Coordinator-Managed Transaction Lifecycle**:
   ```
   INITIATED
       │
       ▼
   PREPARING ──(Transfer packs to primary & replicas)
       │
       ▼
   PREPARED  ──(All required nodes verified object hashes)
       │
       ▼
   COMMIT_DECIDED ──(Coordinator durably records commit record)
       │
       ▼
   COMMITTED ──(Generation advanced, references published to client)
   ```
   - **Alternative Failure States**: `ABORTED` (safely rolled back before commit decision) and `RECOVERY_REQUIRED` (inconsistency detected during execution).

3. **Incremental Pack-Based Replication**:
   - Nodes exchange pack identifiers and generation sequences rather than entire repository archives.
   - The primary builds or receives a new packfile, computes its checksum, and stages it.
   - Replicas fetch only missing `.spk` and `.spidx` files and verify their contents against the advertised generation catalog.
   - Upon verifying all object hashes and index fanouts, replicas issue signed preparation acknowledgements to the coordinator.

4. **Crash Recovery Semantics**:
   - If the coordinator crashes while a transaction is in `PREPARING` or `INITIATED`, restart recovery safely aborts the incomplete transaction.
   - If the coordinator crashes after reaching `COMMIT_DECIDED`, restart recovery guarantees completion of the commit decision across all participating nodes.

## Consequences
- Guaranteed consistency: clients never receive false success acknowledgements for un-replicated commits.
- Zero silent downgrade: if replica availability falls below policy threshold, the operation returns a retryable durability error.
