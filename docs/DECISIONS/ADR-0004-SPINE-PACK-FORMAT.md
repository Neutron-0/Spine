# ADR-0004: Spine Storage Engine V2 — Immutable Packfiles, 256-Bucket Fanout Indexing, and Generational Catalog

## Status
Accepted

## Date
2026-10-31

## Context
In Batch 01 and Batch 02, Spine implemented a direct content-addressed object store where each object (blob, tree, commit, tag) was stored as an individual loose file under `.spine/objects/<xx>/<yy...>`, compressed with Zstandard and validated with BLAKE3.

While loose object storage is optimal for uncommitted writes, fast single-object mutation, and staging concurrency, it degrades rapidly as repositories grow beyond tens of thousands of objects:
1. **Filesystem Inode and Metadata Overhead:** Storing 100,000+ files causes severe filesystem performance degradation, especially on Windows NTFS and macOS APFS, where directory enumerations, MFT (Master File Table) fragmentation, and file lookup overhead dominate execution time.
2. **Disk Block Wastage:** Filesystems allocate disk space in clusters (typically 4 KB). Small trees or commits (100–300 bytes) consume a full 4 KB block each, inflating repository disk footprint by 10x–30x over raw data size.
3. **Random Read I/O Latency:** History traversal (`spine log`, merge base calculation, reachability walks) requires reading thousands of distinct files sequentially, saturating disk queue depth and defeating operating system disk cache read-ahead heuristics.

Git solved this in 1997 with `.pack` and `.idx` files. However, Git's packfile format carries 25+ years of historical debt: variable-length LEB128 integer encoding, ambiguous delta object types (`OBJ_OFS_DELTA` vs `OBJ_REF_DELTA`), SHA-1 assumption baked into index tables, and reliance on unverified multi-pack indexes (`midx`).

Spine requires an original, 64-bit aligned, zero-ambiguity, algorithm-agile binary packfile (`.spk`) and index format (`.spidx`) coupled with a multi-pack generational catalog (`catalog.json`) that supports high-throughput batch writes, lock-free concurrent reads, bounded memory verification, and safe online compaction.

---

## Decision

### 1. File Format Architecture: Two-File Pairing (.spk + .spidx)

Storage Engine V2 establishes two immutable binary file types stored under `.spine/objects/pack/`:
- `<pack_id>.spk`: The physical object packfile containing serialized, compressed object records with cryptographic framing.
- `<pack_id>.spidx`: The fanout binary index allowing $O(\log_2 N)$ random object resolution by 32-byte `ObjectId` without scanning `.spk`.

### 2. Packfile Binary Layout (`.spk`)
Each packfile consists of:
1. **16-byte Header:**
   - Magic: 4 bytes `b"SPPK"` (Spine Pack).
   - Version: `u16` Little-Endian (Format Version 1).
   - Hash Algorithm: `u16` Little-Endian (1 = BLAKE3, 2 = SHA-256).
   - Object Count: `u32` Little-Endian ($N \ge 0$).
   - Header Checksum: 4 bytes (BLAKE3 32-bit truncation of preceding 12 bytes).
2. **Sequence of $N$ Object Records:**
   - Object Type: 1 byte (`0x01` Blob, `0x02` Tree, `0x03` Commit, `0x04` Tag).
   - Compression Mode: 1 byte (`0x00` Raw Uncompressed, `0x01` Zstandard compressed).
   - Reserved Flags: 2 bytes (must be `0x0000`).
   - Uncompressed Length: `u64` Little-Endian ($L_{orig}$). Bounded by 4 GB safety ceiling to prevent out-of-memory denial-of-service on corrupt headers.
   - Stored Length: `u64` Little-Endian ($L_{stored}$).
   - Object ID: 32 bytes (canonical cryptographic `ObjectId` of the uncompressed domain-separated object).
   - Payload Checksum: 16 bytes (BLAKE3 keyed truncation of the stored byte payload).
   - Stored Payload: $L_{stored}$ bytes of compressed or raw object data.
3. **32-byte Trailer:**
   - Magic: 4 bytes `b"SPTR"` (Spine Pack Trailer).
   - Cumulative BLAKE3 Hash: 28 bytes covering the entire preceding packfile content from byte 0 through the end of the last record.

### 3. Pack Index Binary Layout (`.spidx`)
Each pack index provides fast prefix-based random access:
1. **16-byte Header:**
   - Magic: 4 bytes `b"SPIX"` (Spine Pack Index).
   - Version: `u16` Little-Endian (Format Version 1).
   - Hash Algorithm: `u16` Little-Endian (1 = BLAKE3, 2 = SHA-256).
   - Object Count: `u32` Little-Endian ($N$).
   - Header Checksum: 4 bytes.
2. **256-Bucket Fanout Table (1,024 bytes):**
   - 256 sequential `u32` Little-Endian integers.
   - Entry $i$ contains the cumulative number of objects in the pack whose `ObjectId` begins with a byte value $\le i$.
   - Bucket range for prefix byte $b$: index interval $[F[b-1], F[b])$ (with $F[-1] = 0$).
3. **Lexicographically Sorted Object ID Table ($N \times 32$ bytes):**
   - 32-byte `ObjectId`s sorted ascending.
   - Enables fast binary search within the narrow bracket identified by the fanout table.
4. **Parallel Offset and Metadata Table ($N \times 16$ bytes):**
   - Each entry corresponds 1:1 to the sorted Object ID table:
     - Pack Offset: `u64` Little-Endian (byte offset in `.spk` to start of record).
     - Stored Length: `u32` Little-Endian ($L_{stored}$).
     - Uncompressed Length: `u32` Little-Endian ($L_{orig}$).
5. **32-byte Trailer:**
   - Magic: 4 bytes `b"SIXT"`.
   - Cumulative BLAKE3 Checksum: 28 bytes covering the entire index file.

### 4. Generational Multi-Pack Catalog (`catalog.json`)
To coordinate multiple packs without requiring multi-pack index reconstruction on every write:
- `.spine/objects/pack/catalog.json` tracks active packs, generation sequence numbers, total object count, and pack checksums.
- Monotonically increasing generation number increments on every repack, compaction, or GC.
- Atomic updates via `atomic_write_file` ensure readers never observe a torn catalog state.
- Composite reader consults active packs in reverse chronological order (newest packs first), falling back to loose objects under `.spine/objects/`.

### 5. Compaction and Garbage Collection Strategy
- **Repack (`spine repack`):**
  - Scans loose objects and packs.
  - Assembles new consolidated packfile and companion index into temporary files.
  - Verifies full SHA/BLAKE3 object integrity and index fanout before publishing.
  - Updates `catalog.json` atomically.
  - Only after publication and verification are redundant loose objects safely pruned.
- **Garbage Collection (`spine gc`):**
  - Identifies root references: branch heads (`refs/heads/*`), HEAD, tags (`refs/tags/*`), merge state (`MERGE_STATE`), and operation journal undo/redo snapshots.
  - Computes conservative reachability closure.
  - Enforces a 14-day default grace period for unreferenced objects to protect concurrent operations from time-of-check to time-of-use (TOCTOU) deletion races.

---

## Consequences

### Positive
- **Dramatic Inode Reduction:** Consolidates 100,000+ loose files into 1–3 compact packfiles.
- **High Lookup Performance:** 256-bucket fanout reduces search space to $\sim N / 256$ entries; binary search resolves object offset in $\le 8$ iterations.
- **Dramatically Improved History Walks:** Locality of reference speeds up `spine log` and commit traversal by 4x–8x.
- **Space Efficiency:** Zstandard compression combined with elimination of NTFS cluster padding slashes repository size by up to 75%.
- **Resilience:** Dual-layer checksumming (per-record payload verification + full-pack cumulative checksums) detects physical bit rot instantly.

### Negative
- **Read Path Indirection:** Composite object store must query pack indexes before falling back to loose storage.
- **Immutability Invalidation:** Modifying or deleting an object requires creating a new pack and updating the catalog.
