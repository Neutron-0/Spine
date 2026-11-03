# Distributed Repository Write Path

## 1. Single Authoritative Write Pipeline

All repository-modifying actions in the Spine ecosystem—including native `spine push`, remote SPWP wire updates, and Forge Pull Request merges—execute through **one single authoritative write path**: `AuthoritativeRepositoryWriter`.

No secondary, direct-to-disk or uncoordinated write paths are permitted.

```mermaid
sequenceDiagram
    autonumber
    actor Client as Actor (CLI / Forge PR)
    participant Writer as AuthoritativeRepositoryWriter
    participant Coord as CoordinatorStore
    participant Disp as StorageDispatcher
    participant Primary as Primary Storage Node
    participant Replica as Replica Storage Nodes

    Client->>Writer: execute_write(repo_id, actor, updates, pack_bytes)
    Writer->>Writer: Verify Actor Role & Branch Protection
    Writer->>Coord: Fetch Placement, Epoch, Generation & Durability Policy
    Writer->>Writer: Verify Expected Old OID (CAS) & Object Closure
    Writer->>Disp: PreparePush(2PC Phase 1)
    Disp->>Primary: Stage Pack
    Disp->>Replica: Stage Pack
    Primary-->>Writer: Acknowledge Prepared
    Replica-->>Writer: Acknowledge Prepared
    Writer->>Writer: Verify Durability Quorum Satisfied
    Writer->>Disp: CommitPush(2PC Phase 2)
    Disp->>Primary: Finalize Pack & Advance Generation
    Disp->>Replica: Finalize Pack & Advance Generation
    Writer->>Primary: Atomic Reference Update (.lock)
    Writer->>Coord: Advance Committed Generation & Metadata
    Writer-->>Client: AuthoritativeWriteResult (Generation, Durability, Timestamp)
```

## 2. Invariants Enforced at Write Boundary

1. **Actor Permission**: The actor's capability ticket or authenticated session must have sufficient privilege for the operation.
2. **Branch Protection**: Direct pushes to protected branches require Admin privilege unless PR rules are satisfied.
3. **Fencing and Epochs**: Any request carrying an outdated ownership epoch is immediately rejected.
4. **Compare-And-Swap (CAS)**: Target reference tips must match `expected_old_oid`.
5. **Transitive Object Closure**: New commits and trees are recursively walked to ensure zero missing referenced objects.
6. **Durability Quorum**: Packs must be staged on enough nodes to satisfy the repository's `DurabilityPolicy` before commit decision.
