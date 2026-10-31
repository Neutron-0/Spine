# Spine Storage Architecture V2
**Document ID:** ARCH-STORAGE-0002  
**Status:** Accepted Engineering Architecture  

---

## 1. System Overview

Storage Engine V2 transforms Spine from a simple loose-object content store into a tiered, high-performance, resilient storage architecture designed to scale seamlessly from small individual projects to enterprise repositories with millions of objects.

### 1.1 Architectural Layers
```
+-----------------------------------------------------------------------+
|                       Application Logic / CLI                         |
|         (Commit, Log, Status, Diff, Merge, Switch, Tree Walk)         |
+-----------------------------------------------------------------------+
                                   |
                                   v
+-----------------------------------------------------------------------+
|                          ObjectStore Trait                            |
|             read_object(), write_object(), has_object()               |
+-----------------------------------------------------------------------+
                                   |
                                   v
+-----------------------------------------------------------------------+
|                    CompositeObjectStore (Tiered)                      |
|                                                                       |
|  +---------------------------+       +-----------------------------+  |
|  |       Pack Catalog        |       |     Loose Object Store      |  |
|  |  (Generational Index)     |       |  (.spine/objects/xx/yyyy)   |  |
|  +---------------------------+       +-----------------------------+  |
|               |                                                       |
|               v                                                       |
|  +---------------------------+                                        |
|  |   Active Pack Readers     |                                        |
|  |   [Pack 1, Pack 2, ...]   |                                        |
|  |  (Fanout + Binary Search) |                                        |
|  +---------------------------+                                        |
+-----------------------------------------------------------------------+
                                   |
                                   v
+-----------------------------------------------------------------------+
|               Platform Filesystem & Atomic I/O Engine                 |
|       (Windows NTFS / POSIX Ext4 / APFS, Lock-Resilient Renames)      |
+-----------------------------------------------------------------------+
```

---

## 2. Storage Tiers

### Tier 1: Loose Object Store (`.spine/objects/<prefix>/<hash>`)
- **Role:** Fast, lockless staging area for active operations (commits, tree writes, blob staging).
- **Format:** Domain-separated header `SPINE\0<type>\0<len_le>\0` followed by Zstandard-compressed payload.
- **Concurrency:** Every new object is created in a temporary file and atomically renamed into place. File writes are idempotent: two concurrent processes creating the identical blob will write identical bytes to the same destination.

### Tier 2: Packed Object Store (`.spine/objects/pack/`)
- **Role:** High-density, sequential, compressed archive for long-term storage and bulk traversal.
- **Components:**
  - `<pack_id>.spk`: Contiguous byte stream of compressed object records with cryptographic framing.
  - `<pack_id>.spidx`: 256-bucket fanout binary search index.
  - `catalog.json`: Monotonic generation tracker and active pack manifest.
- **Lookup Cost:** 1 memory/disk seek to read index fanout bucket, $\le 8$ comparisons to locate object ID, 1 seek in `.spk` to read and decompress record payload.

---

## 3. Read and Write Path Flow

### 3.1 Object Read (`read_object(id)`)
1. **Tier 2 (Pack Catalog):**
   - Consult `catalog.json` active packs in reverse chronological order (newest packs first, exploiting temporal locality).
   - In each active pack's index, compute prefix byte $b = \text{id}[0]$.
   - Query the 256-bucket fanout table for slice range $[L, H)$.
   - Perform binary search for `id` across the slice.
   - If found:
     - Read the 16-byte record metadata from the index (pack offset, stored size, original size).
     - Seek to `pack_offset` in `.spk`.
     - Read and verify the record header, payload checksum, and decompress payload.
     - Return `(object_type, uncompressed_payload)`.
2. **Tier 1 (Loose Object Store):**
   - If not found in any active pack, calculate loose path `.spine/objects/<b0>/<b1..b31>`.
   - If loose file exists, read, decompress, and return.
   - If missing, return `ObjectStoreError::NotFound(id)`.

### 3.2 Object Write (`write_object(type, payload)`)
- Writes continue to land in **Tier 1 (Loose Storage)** during normal local development commands (`add`, `commit`, `merge`).
- This guarantees $O(1)$ write latency and zero packfile rewriting during interactive editing.
- Migration from Tier 1 to Tier 2 occurs during explicit or background maintenance: `spine repack`.

---

## 4. Concurrency Model and Locking
1. **Lock-Free Reads:** Readers require no locks. Packfiles and indexes are immutable. Reading `catalog.json` uses atomic replacement so a reader either observes generation $G$ or generation $G+1$.
2. **Maintenance Lock:** Compaction (`spine repack`) and garbage collection (`spine gc`) acquire a cooperative advisory repository lock `.spine/LOCK` to serialize maintenance mutations.
3. **Transient Windows File-Lock Handling:** Windows anti-virus and search indexing services hold transient opportunistic read locks. Spine's `atomic_replace_file` utilizes progressive exponential backoff (10ms, 25ms, 50ms) to ensure guaranteed publication without spurious access errors.
