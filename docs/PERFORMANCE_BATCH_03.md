# Spine VCS Performance Report — Batch 03: Storage Engine V2

## Executive Summary
Batch 03 introduced Storage Engine V2: immutable binary packfiles (`.spk`), 256-bucket fanout binary indexing (`.spidx`), generational catalog management (`catalog.json`), high-throughput loose-to-pack repacking (`spine repack`), conservative reachability-based garbage collection (`spine gc`), and composite tiered storage.

All benchmarks were executed on Windows 11 (NTFS, x86_64) using the dedicated release-mode benchmark runner (`cargo run --release --bin benchmark_runner`) with 1 warm-up cycle and 5 measured iterations per operation against official Git (2.4x).

Spine VCS achieved superior performance across all 10 benchmarks, outperforming Git by **1.59x to 17.14x**.

---

## Benchmark Results (Spine VCS vs Official Git)

**Environment:** Windows 11 (x86_64, NTFS) | Rust 1.85+ Release Profile (`opt-level = 3`, LTO)

| Operation | Workload Description | Spine Min | Spine Median | Spine P95 | Git Median | Speedup Ratio | Memory Footprint |
|---|---|---|---|---|---|---|---|
| **spine init** | Clean repository initialization | 20.49 ms | **22.49 ms** | 27.21 ms | 103.74 ms | **4.61x faster** | ~4,200 KB |
| **spine branch create** | Atomic ref write & journal record | 39.53 ms | **43.31 ms** | 45.99 ms | 78.28 ms | **1.81x faster** | ~4,900 KB |
| **spine switch** | 100 modified files worktree update | 134.43 ms | **140.89 ms** | 159.90 ms | 223.70 ms | **1.59x faster** | ~8,200 KB |
| **spine diff** | 100 modified files LCS Myers diff | 51.97 ms | **54.40 ms** | 60.88 ms | 173.51 ms | **3.19x faster** | ~9,400 KB |
| **spine merge (ff)** | Fast-forward merge & worktree sync | 42.42 ms | **45.77 ms** | 50.10 ms | 112.98 ms | **2.47x faster** | ~5,600 KB |
| **spine merge (3-way clean)** | 3-way divergent clean merge | 49.00 ms | **71.73 ms** | 105.56 ms | 138.96 ms | **1.94x faster** | ~7,900 KB |
| **spine undo** | State rollback (journal vs reset) | 26.37 ms | **28.97 ms** | 33.09 ms | 61.97 ms | **2.14x faster** | ~5,100 KB |
| **spine repack** | Consolidate 200 objects into pack + fanout index | 12.73 ms | **14.88 ms** | 15.22 ms | 89.47 ms | **6.01x faster** | ~9,200 KB |
| **spine log (packed)** | Traverse 50 commits from packfile | 19.59 ms | **22.71 ms** | 28.90 ms | 65.10 ms | **2.87x faster** | ~6,400 KB |
| **spine gc** | Reachability analysis and pruning | 19.34 ms | **22.33 ms** | 24.13 ms | 382.58 ms | **17.14x faster** | ~7,100 KB |

---

## Detailed Performance Analysis

### 1. High-Speed Repack (`spine repack`)
- **Median Latency:** 14.88 ms (vs Git `git repack -d` 89.47 ms — **6.01x speedup**).
- **Architectural Factors:**
  - **Single-Pass Stream Packing:** Object records are written sequentially with streaming BLAKE3 hashing.
  - **Incompressible Bypassing:** Small objects (< 64B) or objects where compression does not reduce size bypass the Zstandard encoder, avoiding wasted compression cycles.
  - **Dual-Layer Index Construction:** Building the 256-bucket fanout table requires only one pass ($O(N)$) over sorted object identifiers. Git's pack indexing requires full multi-table reconstruction and delta base offset calculations.

### 2. Random Object & History Traversal (`spine log` on Packed Objects)
- **Median Latency:** 22.71 ms (vs Git `git log -n 50` 65.10 ms — **2.87x speedup**).
- **Architectural Factors:**
  - **Fanout Lookups:** The 256-entry table reduces the binary search space by 99.6% ($N/256$). Binary search resolves exact file offsets in $\le 8$ iterations.
  - **Locality of Reference:** Packing related commit and tree objects sequentially in `.spk` allows the operating system disk cache read-ahead mechanism to preload adjacent objects into memory.

### 3. Reachability Analysis & Garbage Collection (`spine gc`)
- **Median Latency:** 22.33 ms (vs Git `git gc --prune=now` 382.58 ms — **17.14x speedup**).
- **Architectural Factors:**
  - **In-Memory Traversal Closure:** Spine computes the transitive reachability closure directly in memory using native Rust `HashSet` and `VecDeque` without spawning external processes.
  - **Targeted Loose Pruning:** Only loose objects matching the expired grace period threshold are touched, avoiding global directory rescanning.
