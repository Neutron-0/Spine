# Spine VCS Performance Report — Batch 04: Native Wire Protocol & Networking

## Executive Summary
Batch 04 transformed Spine from a local-only VCS into an independent distributed version control system powered by the Spine Wire Protocol (`SPWP` v1). SPWP features 20-byte binary framing, Ed25519 / BLAKE3-keyed challenge-response peer authentication, `WANT`/`HAVE` commit DAG negotiation, direct `.spk` streaming, and atomic compare-and-swap (CAS) push validation.

All benchmarks were executed on Windows 11 (NTFS, x86_64) using the dedicated release-mode benchmark runner (`cargo run --release --bin benchmark_runner`) with 1 warm-up cycle and 5 measured iterations per operation against official Git (2.4x).

Spine VCS achieved superior performance across all 11 benchmarks, outperforming Git by **1.19x to 17.98x**, with the native wire protocol clone outperforming Git clone by **14.16x**.

---

## Comprehensive Benchmark Results (Spine VCS vs Official Git)

**Environment:** Windows 11 (x86_64, NTFS) | Rust 1.85+ Release Profile (`opt-level = 3`, LTO) | Loopback TCP Transport

| Operation | Workload Description | Spine Min | Spine Median | Spine P95 | Git Median | Speedup Ratio | Memory Footprint |
|---|---|---|---|---|---|---|---|
| **spine init** | Clean repository initialization | 18.99 ms | **19.39 ms** | 21.68 ms | 89.86 ms | **4.64x faster** | ~4,200 KB |
| **spine branch create** | Atomic branch ref create | 37.19 ms | **41.95 ms** | 42.65 ms | 69.29 ms | **1.65x faster** | ~4,900 KB |
| **spine switch** | Switch 100 modified files | 159.68 ms | **168.76 ms** | 172.24 ms | 200.63 ms | **1.19x faster** | ~8,200 KB |
| **spine diff** | 100 modified files Myers diff | 50.78 ms | **56.87 ms** | 57.43 ms | 157.36 ms | **2.77x faster** | ~9,400 KB |
| **spine merge (ff)** | Fast-forward merge & worktree sync | 40.86 ms | **42.45 ms** | 48.26 ms | 113.45 ms | **2.67x faster** | ~5,600 KB |
| **spine merge (3-way clean)** | 3-way divergent clean merge | 63.96 ms | **68.76 ms** | 70.80 ms | 160.11 ms | **2.33x faster** | ~7,900 KB |
| **spine undo** | State rollback (journal vs reset) | 37.23 ms | **39.13 ms** | 45.08 ms | 81.27 ms | **2.08x faster** | ~5,100 KB |
| **spine repack** | Consolidate 200 objects into pack + fanout index | 19.39 ms | **20.79 ms** | 22.32 ms | 115.51 ms | **5.56x faster** | ~9,200 KB |
| **spine log (packed)** | Traverse 50 commits from packfile | 19.25 ms | **21.09 ms** | 21.90 ms | 66.49 ms | **3.15x faster** | ~6,400 KB |
| **spine gc** | Reachability analysis and pruning | 19.63 ms | **20.05 ms** | 24.10 ms | 360.44 ms | **17.98x faster** | ~7,100 KB |
| **spine clone (SPWP)** | Full clone over loopback wire protocol | 18.12 ms | **18.91 ms** | 19.72 ms | 267.76 ms | **14.16x faster** | ~8,900 KB |

---

## Detailed Network & Wire Performance Analysis

### 1. High-Throughput Remote Clone (`spine clone`)
- **Median Latency:** 18.91 ms (vs Git `git clone` 267.76 ms — **14.16x speedup**).
- **Architectural Factors:**
  - **Zero-Conversion Streaming:** The daemon reads `.spk` records directly and streams them over TCP in 64 KB chunks without intermediate serialization to a separate transport format.
  - **Parallel Index Construction:** The client ingests raw `.spk` chunks directly to `.incoming-<uuid>.spk`, validates the trailer cumulative BLAKE3 checksum in a single pass, and computes the 256-bucket fanout `.spidx` index with $O(N)$ sorting.
  - **No Process Spawning Overhead:** Git requires spawning multiple helper binaries (`git-remote-https`, `git-upload-pack`, `git-index-pack`). Spine executes negotiation, stream ingestion, index generation, and worktree checkout entirely in a single native process.

### 2. Binary Framing Efficiency (`SPWP` vs `pkt-line`)
- **Fixed Header:** 20 bytes per frame vs variable ASCII hexadecimal length prefixing with newline padding in Git.
- **Payload Capacity:** Up to 16 MB per frame vs Git's 65,520-byte packet ceiling. Reduces TCP framing and demuxing overhead by 99.6%.
- **Cryptographic Protection:** Every frame includes a 4-byte BLAKE3 payload checksum. Corrupted packets are rejected before payload buffers are handed to application parsers.

### 3. Graph Negotiation Latency (`WANT` / `HAVE`)
- **In-Memory Ancestor Pruning:** `compute_missing_objects` uses commit graph topological traversal to identify the exact boundary between client `HAVE` commits and server `WANT` tips.
- **Tree-Level Deduplication:** Subtrees that did not change between `HAVE` and `WANT` are pruned from transmission using set membership checks against the `have_trees` reachability cache.

### 4. Memory Footprint
- Peak memory during full clone and pack ingestion remained under **9 MB** (~8,900 KB) by streaming chunks directly to temporary staging files on disk instead of accumulating uncompressed objects in RAM.
