# Spine Garbage Collection and Reachability Architecture
**Document ID:** SPEC-GARBAGE-COLLECTION-0001  
**Status:** Accepted Engineering Specification  

---

## 1. Overview
In content-addressed storage systems, mutating operations (abandoned commits, rebases, aborted merges, temporary staging updates) inevitably create unreferenced objects. Garbage collection (GC) reclaims disk space by identifying and safely deleting unreferenced objects while rigorously protecting in-flight work and historical recoverability.

---

## 2. Root Reference Enumeration

An object is reachable if and only if it is in the transitive closure of the repository's **Root Reference Set**:

$$\mathcal{R}_{roots} = \mathcal{R}_{branches} \cup \mathcal{R}_{HEAD} \cup \mathcal{R}_{tags} \cup \mathcal{R}_{merge} \cup \mathcal{R}_{journal} \cup \mathcal{R}_{index}$$

1. **Branch Tips ($\mathcal{R}_{branches}$):** All references under `.spine/refs/heads/*`.
2. **Current HEAD ($\mathcal{R}_{HEAD}$):** Direct commit reference if HEAD is detached.
3. **Tags ($\mathcal{R}_{tags}$):** All references under `.spine/refs/tags/*`.
4. **Active Merge State ($\mathcal{R}_{merge}$):**
   - Base commit, ours commit, theirs commit, and all conflict objects recorded in `.spine/MERGE_STATE`.
5. **Operation Journal State ($\mathcal{R}_{journal}$):**
   - All `before_state` and `after_state` commits, root trees, and branch tips referenced in any non-pruned journal record (`.spine/journal/records/*.json`).
   - This ensures that `spine undo` can never fail due to missing historical snapshots.
6. **Binary Index ($\mathcal{R}_{index}$):**
   - All object IDs currently staged in `.spine/index`.

---

## 3. Transitive Closure Traversal Algorithm

Starting from $\mathcal{R}_{roots}$, the GC engine traverses the object graph:
1. For each commit $C \in \mathcal{R}_{roots}$:
   - Mark $C$ as reachable.
   - Add all parent commit IDs $P_i \in C.parents$ to the traversal queue.
   - Add the root tree ID $T_{root} \in C.root\_tree$ to the tree traversal queue.
2. For each tree $T$:
   - Mark $T$ as reachable.
   - For each entry $E \in T.entries$:
     - If $E.mode == FileMode::Directory$: enqueue sub-tree $E.object\_id$.
     - Else: mark blob $E.object\_id$ as reachable.
3. For each tag $G$:
   - Mark tag target $G.target\_id$ as reachable.

---

## 4. Time-of-Check to Time-of-Use (TOCTOU) Protection & Grace Period

Deleting unreferenced objects immediately upon running GC creates severe race conditions:
- A concurrent developer running `spine add <file>` writes loose blobs to disk before creating the commit. A concurrent GC could observe these blobs as "unreferenced" and prune them, causing silent data loss when the commit is subsequently written.
- To prevent this, Spine enforces a **Strict Grace Period** (default: 14 days, configurable via `spine gc --prune-before <duration>`):
  - An unreachable object is eligible for physical deletion ONLY if its filesystem modification time (`mtime`) is older than the grace period threshold:
  $$\text{mtime}(O) < T_{now} - \Delta_{grace}$$
  - Objects written within the last 14 days are preserved unconditionally, even if currently unreachable.

---

## 5. Dry-Run and Storage Verification
- `spine gc --dry-run` simulates the complete reachability traversal, enumerates candidates for pruning, and calculates recoverable disk bytes without performing physical file deletions.
- Post-collection verification (`spine verify-storage`) audits that all reachable objects remain readable, intact, and cryptographically verified.
