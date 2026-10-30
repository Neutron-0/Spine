# ADR-0001: Version-Control Core Data Model & Storage Architecture

## Status
Accepted

## Date
2026-10-31

## Context
When designing a modern, Rust-native distributed version-control system to serve as an alternative to Git, we must select the fundamental data representation for files, directories, commits, and history transitions. Three main paradigms exist:
1. Snapshot-oriented Merkle DAG (Git, Mercurial).
2. Patch-oriented graph algebra (Pijul, Darcs).
3. Operation-journaled working-copy commits (Jujutsu).

The data model directly dictates repository performance, deduplication capability, merge semantics, recovery capabilities, and network replication efficiency.

## Decision
We select a **Snapshot-Oriented, Content-Addressed Merkle DAG** paired with an **Append-Only Operation Journal** and a **Binary Staging Manifest Index**:

1. **Object Identity:** Objects are identified by a 32-byte hash computed via BLAKE3 by default, with optional SHA-256 support.
2. **Strict Domain Separation:** Every object is hashed with a mandatory domain prefix:
   `SPINE\0<type_byte>\0<payload_len_u64_le>\0<payload>`
   This formally prevents type-confusion vulnerabilities where a blob byte sequence matches a serialized tree or commit.
3. **Core Object Types:**
   - `Blob (0x01)`: File content bytes, compressed using Zstandard.
   - `Tree (0x02)`: Canonical sorted list of mode, filename, and target `ObjectId` references.
   - `Commit (0x03)`: Snapshot metadata referencing a root tree, parent commit IDs, author/committer identities, UTC nanosecond timestamps, and commit message.
   - `Tag (0x04)`: Named cryptographic pointer with optional signature.
4. **Staging & Stat Manifest (`spine-index`):** A binary file storing active staged paths, file modes, sizes, timestamps, and cached object IDs. Unchanged files bypass disk re-reads.
5. **Atomic Operations:** Reference updates and loose object writes employ write-to-tempfile followed by atomic rename, guaranteeing crash consistency across Windows, Linux, and macOS.

## Consequences
- **Positive:**
  - Fast, deterministic commit verification and constant-time integrity checks.
  - Zero-overhead deduplication across identical files and directories.
  - High decompression throughput with Zstandard.
  - Intuitive mental model compatible with industry developer expectations.
  - Full crash resilience against interrupted writes or system crashes.
- **Negative / Mitigations:**
  - Tree-diff computation is required to derive changes between snapshots (mitigated by caching tree hashes in index).
  - Storage growth for loose objects (mitigated in Milestone 3 via packfiles and delta compression).
