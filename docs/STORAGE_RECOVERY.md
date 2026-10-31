# Spine Storage Recovery and Durability Specification
**Document ID:** SPEC-STORAGE-RECOVERY-0001  
**Status:** Accepted Engineering Specification  

---

## 1. Overview
In distributed and local version control systems, system crashes, power loss, process termination, and storage hardware errors can occur at any arbitrary instruction. Storage Engine V2 provides rigorous durability guarantees that ensure repository state is never left corrupted, unrecoverable, or inconsistent.

---

## 2. Five-State Operation Journal State Machine

To coordinate multi-step mutations across loose objects, packfiles, indices, reference heads, and the working tree, the operation journal enforces a five-state transition model:

```
                  +--------------+
                  |   Prepared   |
                  +--------------+
                     |        \
    (mutations begin)|         \ (interrupted before apply)
                     v          v
              +------------+   +-----------+
              |  Applying  |   |  Aborted  |
              +------------+   +-----------+
                 |        \          ^
    (all succeed)|         \         | (rolled back clean)
                 v          v        |
           +-----------+   +------------------+
           | Committed |   | RecoveryRequired |
           +-----------+   +------------------+
```

### State Definitions:
1. **`Prepared`:** The operation intent and `before_state` (HEAD, branch references, index checksum) are durably flushed to `.spine/journal/records/<seq>-<txid>.json`. No filesystem or object mutations have occurred yet.
2. **`Applying`:** The transaction has commenced applying mutations (e.g. writing new packfiles, modifying references, or updating working-tree files).
3. **`Committed`:** All repository mutations have completed and flushed. The transaction record status is updated to `Committed`, and `.spine/journal/HEAD` is atomically updated to point to this transaction.
4. **`Aborted`:** The operation failed or was interrupted, and the repository was cleanly and safely rolled back to `before_state`.
5. **`RecoveryRequired`:** An interrupted transaction encountered an unrecoverable failure during automatic rollback (e.g., persistent hardware disk error or permission denial). The engine fails closed, halts further mutations, and alerts the user.

---

## 3. Idempotent Crash Recovery Protocol

Whenever `spine fsck` or any mutating Spine command executes:
1. The journal engine scans `.spine/journal/records/` for any records in `Prepared` or `Applying` status.
2. For each unfinalized record:
   - If in `Prepared`: No mutations were completed; safely mark `Aborted` with reason `"Interrupted prior to application"`.
   - If in `Applying`: Execute verified rollback to `before_state`:
     - Revert branch reference and HEAD to recorded snapshots.
     - Restore binary index from snapshot.
     - If rollback succeeds: update status to `Aborted` with reason `"Interrupted by crash; rolled back to before_state"`.
     - If rollback fails: update status to `RecoveryRequired` with detailed diagnostic error.
3. **Idempotency Guarantee:** If recovery is executed multiple times in succession, the second and subsequent invocations detect zero unfinalized records, perform no I/O mutations, and exit cleanly with `Ok(None)`.

---

## 4. Packfile Publication Protocol (Zero-Data-Loss Invariant)

To guarantee that objects are never lost during repacking or garbage collection, Storage Engine V2 enforces the **Strict Staged Publication Invariant**:

```
[ Step 1: Write & Fsync Temp Pack ]  --> .tmp-pack-<id>.spk
[ Step 2: Write & Fsync Temp Index ] --> .tmp-pack-<id>.spidx
[ Step 3: Atomic Rename ]            --> <id>.spk & <id>.spidx
[ Step 4: Atomic Catalog Publish ]   --> catalog.json (Generation G -> G+1)
[ Step 5: Full Integrity Audit ]     --> Verify all objects in new pack
[ Step 6: Safe Pruning ]             --> Only now delete redundant loose objects
```

### Safety Properties:
- **Crash before Step 4:** The new `.spk` and `.spidx` exist on disk but are not listed in `catalog.json`. All readers continue reading from existing packs and loose objects without interruption. Orphan temp files are cleaned up during subsequent GC.
- **Crash during Step 4:** `catalog.json` uses atomic rename with fsync. The OS guarantees that `catalog.json` either contains generation $G$ or $G+1$; torn reads are physically impossible.
- **Verification Gate (Step 5):** Loose objects are NEVER removed unless the new packfile and index pass full checksum verification. Even in the event of catastrophic disk corruption during pack generation, the original loose objects remain untouched.
