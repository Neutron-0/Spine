# SPINE PERFORMANCE STRATEGY

## 1. Performance Ambition & Methodology

Spine's engineering goal is to deliver deterministic, high-throughput version control operations that match or outperform existing Git tooling on critical local workflows:
- Rapid repository initialization and configuration.
- High-efficiency file staging with parallel directory traversal and metadata hashing.
- Ultra-low latency status checks accelerated by an in-memory stat-manifest cache.
- Fast commit graph traversal and topological sorting.
- High compression throughput with Zstandard stream frames.
- Low-memory deep-history walks.

Performance claims must never be speculative; they must be backed by reproducible benchmark suites using Criterion and realistic multi-scale synthetic datasets.

---

## 2. Algorithmic and Structural Design

### 2.1 Hashing: BLAKE3 vs SHA-256
- **BLAKE3:** Primary cryptographic hashing algorithm for Spine VCS.
  - Tree-based structure enables multithreaded SIMD-accelerated parallel hashing.
  - Achieves >3–6 GB/s throughput per core on modern x86-64 processors with AVX-512/AVX2.
  - 4–8x faster than SHA-256 and SHA-1 while maintaining 128-bit security level against collision and preimage attacks.
- **SHA-256:** Retained as an optional fallback/verification algorithm to ensure compatibility and compliance with legacy policies.

### 2.2 Compression: Zstandard (zstd)
- Eliminates legacy zlib deflate bottlenecks.
- Decompression speeds exceed 1.2 GB/s per core, enabling near-instantaneous checkout and object inspection.
- Tunable compression levels (level 1–3 for interactive operations; level 19 for packfile compaction).

### 2.3 Incremental Filesystem Indexing (`spine-index`)
- Similar to Git's `index`, the Spine staging index maintains file metadata (`mtime`, `size`, `inode`, `mode`, `object_id`).
- Running `spine status` executes a lightweight stat pass:
  1. Stat working directory file.
  2. If `(mtime_secs, mtime_nanos, size)` matches the cached index entry, skip reading file contents entirely.
  3. If changed, stream file through BLAKE3, compare computed `ObjectId`, and update index state.
- Reduces status check time from $O(\text{total bytes})$ to $O(\text{number of files})$.

### 2.4 Atomic Tempfile and Windows Buffer Strategies
- On Windows, file I/O operations must be carefully staged:
  - Buffer sizes tuned to 64KB for streaming file reads.
  - Temporary files created in `.spine/tmp/` on the same filesystem volume, enabling instantaneous atomic rename.
