# RESEARCH NOTE 03: JUJUTSU (JJ) ARCHITECTURE & OPERATION JOURNAL ANALYSIS

## 1. Core Model & Philosophy
Jujutsu reimagines VCS workflow with an operation-log model:
- **Automatic Working Copy Commits:** The working copy is always an automatic commit. Changing a file immediately updates the current commit object without requiring explicit `git add`.
- **First-Class Conflicts:** Merge conflicts do not block operations or leave markers in working files; instead, conflict states are recorded directly in the tree object as a multi-way snapshot.
- **Operation Log (`op_store`):** Every mutation (rebase, commit, branch switch) is an append-only operation in `.jj/repo/op_store/`. This provides native, lossless `undo` and `redo`.
- **Anonymous Branches:** Branches are optional pointers; commits are identified by change IDs that persist across rebases.

## 2. Key Subsystems Inspected
- `lib/src/op_store.rs`: Serializes operations with parent links, timestamps, and view states.
- `lib/src/working_copy.rs`: Snapshotting engine watching working directory changes.
- `lib/src/dag_walk.rs`: Algorithms for finding merge ancestors and computing reachability.

## 3. What Spine Adopts
1. **Operation Journaling Concept:** Recording atomic repository state transitions in an append-only journal (`.spine/journal/`) to guarantee instantaneous undo/redo in future milestones.
2. **First-Class Conflict Modeling:** Designing tree entry schemas capable of encoding conflict states cleanly.

## 4. What Spine Avoids
1. **Continuous Implicit Commits:** JJ's model of committing on every keystroke/command adds substantial background I/O overhead. Spine maintains an intentional, explicit staging index (`spine add` / `spine commit`) optimized for developer precision and predictable performance.
