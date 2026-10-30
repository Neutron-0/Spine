# COMPARATIVE ANALYSIS OF VERSION-CONTROL MODELS

This engineering document evaluates the three principal version-control architectural models:
1. **Snapshot-Oriented Merkle DAG (Git / Spine Primary)**
2. **Patch-Oriented Algebra (Pijul / Darcs)**
3. **Operation / History Journaling (Jujutsu)**

---

## 1. Architectural Model Comparison

| Dimension | Snapshot-Oriented Merkle DAG (Git / Spine) | Patch-Oriented Algebra (Pijul) | Operation / History Journaling (Jujutsu) |
|---|---|---|---|
| **Primary Abstraction** | Immutable state snapshots (Blobs + Trees + Commits) | Commuting patches forming a graph of lines | First-class working-copy commits + Operation log |
| **Identity of Changes** | Derived post-hoc via diffing snapshots | First-class cryptographic patch hash | Persistent Change-ID independent of commit hash |
| **Deduplication** | Content-addressed SHA-256 / BLAKE3 hashes of blobs and subtrees | Content deduplication inside Sanakirja B-Tree | Merkle tree snapshots cached in backing store |
| **Merge Resolution** | Three-way merge (LCA computation + diff application) | Category-theoretic patch composition ($A \cdot B = B \cdot A$) | Multi-way snapshot tree with first-class conflict objects |
| **Conflict Representation** | Ephemeral conflict markers in working copy | Non-linear cycle/ambiguity in file line graph | Conflict object stored directly in repository tree |
| **History Recovery** | Reflog (lossy, non-deterministic timeline) | Inherent patch dependencies | Append-only operation journal (`op_store`) |
| **Storage Complexity** | Compact with delta packfiles; $O(\text{unique content})$ | Higher overhead due to line-level graph topology | Comparable to Git snapshots + lightweight operation log |
| **Algorithmic Complexity** | Well-understood ($O(N)$ tree walk, $O(ND)$ Myers diff) | Highly complex; potential $O(N^2)$ graph linearization | Moderate; operation tree resolution + commit DAG |

---

## 2. In-Depth Evaluation

### 2.1 Snapshot-Oriented Storage (Git / Spine VCS)
- **Mechanics:** Every commit points to an immutable root tree representing the entire state of the repository at that instant. Unchanged subtrees and files share identical hashes and are stored only once.
- **Advantages:**
  - Fast, constant-time commit verification.
  - Highly efficient checkout and diffing between distant commits.
  - Excellent compressibility with Zstandard and delta compression.
  - Transparent, battle-tested mental model for developers.
- **Limitations:**
  - Branch renames and rebases rewrite commit hashes, obscuring original intention.
  - Traditional Git reflog is local and impermanent.

### 2.2 Patch-Oriented Storage (Pijul)
- **Mechanics:** Changes are patches with explicit dependencies. If patch $A$ does not depend on patch $B$, they commute. Merges are commutative and associative.
- **Advantages:**
  - Resolves merge order dilemmas cleanly.
  - Cherry-picking is mathematically natural without "cherry-pick commit duplicates".
- **Limitations:**
  - Managing file lines as graph vertices adds tremendous overhead to basic operations.
  - Edge cases in line graphs can produce cyclic dependencies or ambiguous ordering.
  - Lacks industry tooling, mature packfile ecosystems, or high-throughput distributed sync.

### 2.3 Operation / History Journaling (Jujutsu)
- **Mechanics:** Every mutation of working copy or refs writes an append-only operation to `op_store`. Commits carry persistent `Change-ID`s distinct from their cryptographic commit IDs.
- **Advantages:**
  - Lossless `undo` and `redo` across all operations.
  - Conflicts can be checked into branches without blocking progress.
- **Limitations:**
  - Continuous automatic commit generation creates high filesystem I/O load on large repositories.
  - Increases working-copy lock contention on Windows systems.

---

## 3. The Spine VCS Decision

**Decision:** Spine adopts a **Snapshot-Oriented, Content-Addressed Merkle DAG** as its primary storage foundation, enhanced with:
1. **Domain-Separated BLAKE3 Hashing:** Guarantees cryptographic collision resistance and parallel throughput.
2. **Append-Only Operation Journaling:** Storing atomic state transitions in `.spine/journal/` for reliable undo/recovery.
3. **High-Performance Staging Manifest (`spine-index`):** Caching filesystem stat attributes to deliver sub-millisecond status checks.
4. **Structured Tree Conflict Modeling:** Enabling future rich conflict storage in tree nodes without corrupting plain file data.

This hybrid approach achieves the robust simplicity of Merkle snapshots with the safety and recoverability of modern operation journals, avoiding the unproven complexity of patch algebras.
