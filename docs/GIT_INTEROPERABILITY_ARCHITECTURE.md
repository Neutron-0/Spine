# GIT INTEROPERABILITY ARCHITECTURE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. ARCHITECTURAL MISSION & PHILOSOPHY

Spine is an independent, Rust-native distributed version-control system and code collaboration platform. Spine does not clone Git's codebase, wrap `libgit2`, nor execute external Git CLI binaries in production.

However, developer ecosystems operate under strong network effects. To allow teams to seamlessly transition existing Git repositories into Spine Forge—and to ensure zero vendor lock-in by providing deterministic export from Spine into Git—Spine Batch 15 introduces a **pure Rust native Git interoperability engine** in `git-alternative/crates/spine-migration`.

### Fundamental Architecture Invariants

1. **Zero External Binaries in Production:**  
   Spine executes no subshells, invokes no `git` CLI, and links against no foreign C libraries like `libgit2`. All Git format parsing and serialization is implemented in pure memory-safe Rust.
2. **Oracle Isolation:**  
   The official Git CLI (`git fsck --full`, `git log`, `git cat-file`) is utilized exclusively within unit and integration test suites as an external validation oracle.
3. **Lossless Object Topology:**  
   Spine maintains an immutable, bidirectionally deterministic mapping between Git object identifiers (20-byte SHA-1 or 32-byte SHA-256) and Spine object identifiers (32-byte BLAKE3), preserving parent topologies, author timestamps, commit messages, and tree hierarchies.
4. **Resumable and Idempotent Pipelines:**  
   All migration workflows operate through an inspection, planning, phased execution, checkpoint manifest, and verification pipeline. Any migration interrupted by power loss or process kill can be resumed without data corruption or redundant object conversion.

---

## 2. ENGINE SUBSYSTEM OVERVIEW

```
+-----------------------------------------------------------------------------------+
|                                  SPINE FORGE                                      |
|   Frontend Migration UI (React)  <--->  Backend Migration API (Axum)              |
|                                         CLI Commands (spine import / export)     |
+-----------------------------------------------------------------------------------+
                                          |
                                          v
+-----------------------------------------------------------------------------------+
|                                spine-migration                                    |
|                                                                                   |
|  +---------------------------+  +-----------------------------------------------+ |
|  |     Git Format Engine     |  |          Archive & Security Engine            | |
|  | - Loose Object Parser/Enc |  | - Zip Slip & Path Traversal Prevention        | |
|  | - Canonical Tree Sorter   |  | - Windows Reserved Device Name Blocker        | |
|  | - Commit & Tag Serializer |  | - Decompression Bomb Defense (Ratio & Size)  | |
|  | - Packfile v2 & Delta Res |  | - Workflow Deactivator (Fail-Closed)          | |
|  | - Pack Builder & v2 Index |  | - Safe Attribution & Ghost User Mapper        | |
|  | - Packed-Refs & HEAD Mgr  |  | - Secret Scanner & Redactor                   | |
|  +---------------------------+  +-----------------------------------------------+ |
|                                                                                   |
|  +---------------------------+  +-----------------------------------------------+ |
|  |     Migration Engine      |  |          Fast Stream & LFS Interop            | |
|  | - Inspect & Plan          |  | - Fast-Import Stream Parser                   | |
|  | - Topological Importer    |  | - Fast-Export Stream Generator                | |
|  | - CAS Traversal Exporter  |  | - Git LFS Pointer Parsing & Verification      | |
|  | - Checkpoint Manifests    |  | - Spine CAS Artifact Hydration                | |
|  +---------------------------+  +-----------------------------------------------+ |
+-----------------------------------------------------------------------------------+
           |                                                      |
           v                                                      v
+-----------------------+                              +-----------------------+
|   Native Spine Repo   |                              |  Standard Git Repo    |
|   (.spine/ storage)   |                              |  (.git/ loose & pack) |
+-----------------------+                              +-----------------------+
```

---

## 3. CORE PROCESSING PIPELINES

### 3.1 Git Import Pipeline (`GitImporter`)

1. **Repository Layout Discovery:**  
   Detects whether the target is a bare Git repository (`HEAD`, `objects/`, `refs/`) or working tree (`.git/` subdirectory).
2. **Object Ingestion:**  
   - Inspects `objects/pack/`: Parses `.idx` v2 fanout tables, indexes, and `.pack` streams. Solves `OFS_DELTA` and `REF_DELTA` chains into materialized Git objects.
   - Inspects `objects/??/`: Inflates loose zlib-compressed objects and calculates Git SHA-1 hashes.
3. **Reference Resolution:**  
   Reads both loose references (`refs/heads/*`, `refs/tags/*`) and packed references (`packed-refs`), identifying all reachable branch heads and annotated tags.
4. **Bottom-Up Conversion:**  
   - **Blobs:** Unpacks Git blobs and stores them directly into Spine's content-addressed object store (`LooseObject::Blob`).
   - **Trees:** Parses Git tree entries, maps each child Git SHA-1 to its corresponding Spine `ObjectId`, and writes Spine tree objects.
   - **Commits:** Topologically orders commits from roots to tips. Re-maps parent commits, tree pointers, authors, committers, and messages into Spine commits.
   - **Tags:** Converts Git annotated tags to Spine tags, preserving cryptographic PGP/GPG signatures where present.
5. **Reference & Working Copy Sync:**  
   Updates Spine's `.spine/refs/` directory and performs an initial clean checkout of `HEAD` into the working directory.

### 3.2 Git Export Pipeline (`GitExporter`)

1. **Commit Topology Traversal:**  
   Gathers all Spine commits reachable from branches and tags.
2. **Object Generation:**  
   - Reads Spine blobs and formats them as standard Git blobs: `blob <len>\0<content>`.
   - Reads Spine trees, converts entries to standard Git tree entries, and applies canonical Git directory sorting (sorting directory entries with synthetic trailing `/`).
   - Translates Spine commits to standard Git commit format (`tree <oid>\nparent <oid>\n...`).
3. **Storage Emission:**  
   - For small repositories or incremental updates: Writes loose objects into `.git/objects/??/` compressed with zlib.
   - For bulk export: Encodes objects into a single compressed Git Packfile v2, computes offsets, and writes a companion `.idx` v2 file with a 256-entry fanout table.
4. **Reference and HEAD Setup:**  
   Writes `.git/refs/heads/*`, `.git/refs/tags/*`, and `.git/HEAD`.
5. **Working Tree Materialization:**  
   Checks out the target branch files into the working directory.

---

## 4. SECURITY & ROBUSTNESS GUARANTEES

- **Memory Protection:** Zero unsafe code in parser paths.
- **Archive Boundary Protection:** Multi-layer traversal defense rejecting paths attempting to escape target roots via `../` or absolute drive prefixes.
- **Windows System Integrity:** Explicit blocking of Windows reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`).
- **Decompression Bomb Protection:** Hard ceilings on uncompressed archive byte sizes, maximum file counts, and compression ratios (> 100:1 rejected).
- **Fail-Closed Workflow Policy:** Workflows migrated from external platforms (GitHub Actions, GitLab CI) are imported with `is_active: false` to prevent untrusted remote execution.
