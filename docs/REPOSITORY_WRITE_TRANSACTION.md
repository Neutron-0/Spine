# Repository Write Transactions (Two-Phase Commit)

## 1. Transaction Lifecycle

Spine distributed write transactions follow a strict two-phase commit protocol managed by `AuthoritativeRepositoryWriter` and tracked in `CoordinatorStore`.

```
[Init]
  │
  ▼
[Staged] ──(Prepare on Primary & Replicas)──► [Quorum Met?]
                                                ├── No ──► [Aborted] (Rollback Staging)
                                                └── Yes ─► [Commit Decided]
                                                                │
                                                                ▼
                                                            [Committed] (Finalize Packs & Refs)
```

## 2. Durability Policies

The cluster supports three durability policies:
- `SingleCopy`: Requires 1 verified copy (local primary). Suitable for scratch repos.
- `TwoCopies`: Requires primary + at least 1 replica (2 copies total). Standard durability.
- `ThreeCopies`: Requires primary + 2 replicas (3 copies total). High durability enterprise policy.

## 3. Transaction State Representation

Transactions are recorded in the coordinator with:
- `transaction_id`: Unique identifier (`tx_<uuid>`).
- `repo_id`: Target repository.
- `ownership_epoch`: Generation epoch of primary node.
- `proposed_generation`: Monotonically increasing generation number.
- `pack_checksum`: BLAKE3 checksum of the packfile.
- `prepared_nodes`: Set of storage node IDs that have successfully staged the pack and indices.
- `status`: `Prepared`, `CommitDecided`, `Committed`, or `Aborted`.
- `created_at_nanos` / `expires_at_nanos`: Expiry threshold to recover from coordinator or node crashes.
