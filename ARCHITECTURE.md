# SPINE ARCHITECTURAL SPECIFICATION

## 1. System Vision & Boundaries

SPINE is designed as an end-to-end version control and collaboration platform divided cleanly into two independent yet complementary tiers:

1. **Spine VCS (`git-alternative/`):**
   - High-performance, Rust-native distributed version-control engine.
   - Operates fully decentralized, offline-first, with native CLI tooling (`spine`).
   - Owns the physical on-disk format (`.spine/`), content-addressed object store, working tree index, commit Merkle DAG, diffing engines, and integrity verification.
   - Zero reliance on external Git binaries or C-library bindings (`libgit2`).

2. **Spine Forge (`github-alternative/`):**
   - Developer collaboration platform, code hosting, repository management, pull requests, issues, and distributed storage nodes.
   - Backend built with Rust (Axum, Tokio, SQLx/PostgreSQL).
   - Distributed object storage nodes that manage physical Spine repositories and provide high-availability replication.
   - Frontend built with React, TypeScript, and Vite.
   - Strictly consumes Spine VCS via library APIs and RPC/REST interfaces.

```
+-------------------------------------------------------------------------+
|                              SPINE FORGE                                |
|   +-----------------------+              +--------------------------+   |
|   |   Web UI (React+TS)   |  <--HTTP/WS--|   Spine API (Axum+Tokio) |   |
|   +-----------------------+              +--------------------------+   |
|                                                       |                 |
|                                              PostgreSQL (Metadata)      |
|                                                       |                 |
|                                          +--------------------------+   |
|                                          |   Spine Storage Node     |   |
|                                          +--------------------------+   |
+-------------------------------------------------------|-----------------+
                                                        | Rust Library API
+-------------------------------------------------------v-----------------+
|                               SPINE VCS                                 |
|   +-----------------------------------------------------------------+   |
|   |                        CLI (`spine`)                            |   |
|   +-----------------------------------------------------------------+   |
|   | spine-integrity | spine-history | spine-diff   | spine-worktree |   |
|   +-----------------+---------------+--------------+----------------+   |
|   | spine-index     | spine-config  | spine-format | spine-core     |   |
|   +-----------------------------------------------------------------+   |
|   |                    spine-object-store (CAS)                     |   |
|   +-----------------------------------------------------------------+   |
|   |                       On-Disk Engine                            |   |
|   |       .spine/objects/ (BLAKE3 CAS, Zstd)                        |   |
|   |       .spine/refs/    (Atomic transactional references)         |   |
|   |       .spine/index    (Binary staging manifest)                 |   |
|   +-----------------------------------------------------------------+   |
+-------------------------------------------------------------------------+
```

---

## 2. Spine VCS Data Model & Storage

### 2.1 Content-Addressed Merkle DAG
Every artifact in a Spine repository is stored as an immutable object identified by its cryptographic digest:
- **Object ID (ObjectId):** 32-byte BLAKE3 hash (with SHA-256 compatibility support).
- **Domain Separation:** To prevent cross-type collision attacks (e.g., crafting a blob that parses as a valid tree), every object's payload is hashed with a strict domain separator header: `SPINE\0<type_byte>\0<payload_len_u64_le>\0`.

### 2.2 Core Object Types
1. **Blob (`0x01`):** Raw byte streams representing file contents. Stored compressed with Zstandard.
2. **Tree (`0x02`):** Canonical sorted list of directory entries. Each entry contains mode flags (executable, regular, symlink), filename, and target `ObjectId`.
3. **Commit (`0x03`):** Snapshot metadata linking a root `Tree`, zero or more parent commit `ObjectId`s, author signature, committer signature, timestamp (UTC nanoseconds), and commit message.
4. **Tag (`0x04`):** Annotated cryptographic reference to an object.

### 2.3 Repository Layout (`.spine/`)
```
.spine/
├── spine.toml          # Repository configuration and version metadata
├── HEAD                # Reference pointer (e.g. "ref: refs/heads/main")
├── index               # Binary index tracking working tree and staged state
├── objects/
│   ├── info/           # Packfile indices and alternate store descriptors
│   ├── [0-9a-f]{2}/    # Loose content-addressed storage (first 2 hex digits)
│   │   └── [0-9a-f]{62}# Zstd-compressed object payload
│   └── pack/           # Compressed multi-object packfiles (future batch)
└── refs/
    ├── heads/          # Local branches (e.g., refs/heads/main)
    └── tags/           # Named tags
```

---

## 3. High-Performance Filesystem & Index Architecture

### 3.1 Binary Staging Manifest (`spine-index`)
The index acts as the staging area between the working directory and the commit history.
- Structured with binary fixed-width entries and length-prefixed path strings.
- Caches file metadata (`mtime_secs`, `mtime_nanos`, `file_size`, `inode`, `mode`, `object_id`).
- Enables $O(1)$ stat-check comparison for unmodified files during `spine status`, avoiding re-hashing unmodified files.

### 3.2 Atomic Filesystem Operations
- **Windows File Lock Immunity:** Windows places mandatory byte locks on open file handles. Spine avoids in-place file mutation for objects and refs.
- **Tempfile Swap:** All object and ref writes are performed to `.spine/tmp/<uuid>`, flushed to physical disk (`fsync`), and atomically renamed via `std::fs::rename` (which invokes `SetFileInformationByHandle` with `FILE_RENAME_POSIX_SEMANTICS` on modern Windows).

---

## 4. Spine Forge Platform Architecture

### 4.1 Backend (`github-alternative/backend`)
- **Technology Stack:** Rust, Axum, Tokio, SQLx.
- **Core Domain Services:**
  - `spine-auth`: JWT, session management, Argon2id password hashing, personal access tokens (PAT).
  - `spine-repositories`: Repository metadata, visibility rules, branch protection policies.
  - `spine-storage-node`: Decentralized daemon hosting local repositories, serving CAS chunks and RPCs.
  - `spine-api`: Unified REST/JSON API for frontend consumption.

### 4.2 Frontend (`github-alternative/frontend`)
- **Technology Stack:** TypeScript, React, Vite, Tailwind CSS / modern CSS modules.
- **Capabilities:** Repository exploration (source tree browser, commit graph visualization, diff viewer), issue tracker, pull request review suite, settings management.
