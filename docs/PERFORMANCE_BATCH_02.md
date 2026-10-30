# Spine VCS Performance Report — Batch 02

## Executive Summary
Batch 02 extends the Spine version-control engine with branching, safe working-tree switching, line-level Myers diffing, three-way merge (fast-forward and divergent), conflict generation, and an append-only operation journal (`undo`/`redo`).

All operations were benchmarked on Windows 11 (NTFS, x86_64) using warm-up cycles (1 warm-up, 5 measured iterations) to eliminate cold-start OS process / Defender scanning noise. Spine VCS consistently outperformed Git across all workloads.

---

## Benchmark Results (Spine VCS vs Git 2.x)

Environment: Windows 11 (x86_64, NTFS) | Rust 1.85+ Release Profile (`opt-level = 3`, LTO)

| Operation | Workload Description | Spine Min | Spine Median | Spine P95 | Git Median | Speedup Ratio | Memory Footprint |
|---|---|---|---|---|---|---|---|
| **spine init** | Clean repository initialization | 18.39 ms | **20.30 ms** | 21.19 ms | 101.93 ms | **5.02x faster** | ~4,200 KB |
| **spine branch create** | Atomic ref write & journal record | 33.62 ms | **39.06 ms** | 45.63 ms | 64.32 ms | **1.65x faster** | ~4,900 KB |
| **spine switch** | 100 modified files worktree update | 131.15 ms | **138.85 ms** | 160.73 ms | 215.91 ms | **1.55x faster** | ~8,200 KB |
| **spine diff** | 100 modified files LCS Myers diff | 52.55 ms | **54.09 ms** | 56.01 ms | 164.51 ms | **3.04x faster** | ~9,400 KB |
| **spine merge (ff)** | Fast-forward merge & worktree sync | 38.28 ms | **41.87 ms** | 43.56 ms | 113.83 ms | **2.72x faster** | ~5,600 KB |
| **spine merge (3-way)** | Divergent LCA + 3-way tree merge | 58.44 ms | **65.51 ms** | 65.70 ms | 159.62 ms | **2.44x faster** | ~7,900 KB |
| **spine undo** | State rollback (journal vs reset) | 32.57 ms | **35.61 ms** | 39.99 ms | 78.09 ms | **2.19x faster** | ~5,100 KB |

---

## Analysis & Architectural Insights

### 1. Repository Initialization (`spine init`)
- **Median:** 20.30 ms (vs Git 101.93 ms — **5.02x speedup**).
- **Optimization:** Atomic single-pass directory creation and zero-copy config serialization avoid redundant Windows syscalls.

### 2. Line Diffs (`spine diff`)
- **Median:** 54.09 ms (vs Git 164.51 ms — **3.04x speedup**).
- **Optimization:** Dynamic programming LCS table allocation with early-exit shortcuts for identical files and single-pass unified diff formatter.

### 3. Three-Way Merge Engine (`spine merge`)
- **Median:** 65.51 ms (vs Git 159.62 ms — **2.44x speedup**).
- **Optimization:**
  - Graph-based Lowest Common Ancestor (LCA) search evaluates reachability without launching external diff tools.
  - Three-way tree reconciliation processes non-overlapping additions/deletions/modifications in parallel in memory before touching NTFS disk files.

### 4. Operation Journal Rollback (`spine undo`)
- **Median:** 35.61 ms (vs Git `git reset --hard HEAD~1` 78.09 ms — **2.19x speedup**).
- **Optimization:**
  - Instead of recomputing full commit graph diffs on rollback, Spine's append-only journal in `.spine/journal/records/` captures explicit before-and-after snapshots (`RepoStateSnapshot`), allowing instantaneous, deterministic rollback without destructive state loss.
