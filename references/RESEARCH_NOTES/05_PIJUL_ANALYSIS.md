# RESEARCH NOTE 05: PIJUL & PATCH THEORY ANALYSIS

## 1. Core Model & Philosophy
Pijul models version control not through snapshots, but through a category-theoretic algebra of patches:
- **First-Class Patches:** History is a set of commuting patches. If two branches make non-overlapping changes, their patches commute ($A \cdot B = B \cdot A$), meaning merge order is mathematically irrelevant.
- **Grayscale Graph of Lines:** Files are represented as directed acyclic graphs of lines (or byte chunks). Adding or deleting lines creates vertices and directed edges. Conflicts are modeled as cycles or multiple conflicting outgoing edges in the graph.
- **Sanakirja Key-Value Engine:** Uses an on-disk copy-on-write B-tree database to store the file graph.

## 2. Key Subsystems Inspected (pijul-core)
- `change.rs`: Encoding of inverse patches, hunk headers, and cryptographic signatures.
- `graph.rs`: Line graph traversal, alive-flag propagation, and conflict detection.

## 3. What Spine Adopts
1. **Mathematical Invariants:** Rigorous validation of invariants (e.g. deterministic tree sorting, cycle detection).
2. **Explicit Dependency Tracking:** Retaining parent commit hashes and tree state lineage.

## 4. What Spine Avoids
1. **Full Graph Representation of All File Lines:** Pijul's line-graph representation incurs significant storage and memory overhead compared to snapshot Merkle trees. File line graphs are computationally expensive to re-linearize ($O(V+E)$ with large constants) and prone to exponential slowdowns on complex history rebases.
2. **Exotic B-Tree Dependencies:** Spine uses pure content-addressed file objects with standard directory fanout (`objects/xx/yy...`), ensuring predictable zero-dependency filesystem performance.
