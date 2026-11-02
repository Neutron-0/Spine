# Spine VCS Batch 06 Performance Engineering Report

## Executive Summary
Batch 06 measured the local developer experience across advanced history manipulation, recovery, stashing, and multi-worktree operations. Benchmarks were conducted against official Git 2.47 on Windows 11 (NTFS, 64-bit).

## Comprehensive Benchmark Results

| Operation | Workload Description | Spine Min | Spine Median | Spine P95 | Git Median | Relative Speedup | Memory RSS |
|---|---|---|---|---|---|---|---|
| **spine init** | Warm repository initialization | 18.85 ms | **19.42 ms** | 19.47 ms | 110.00 ms | **5.66x** | ~4.2 MB |
| **spine branch create** | Atomic branch ref creation | 39.45 ms | **40.01 ms** | 46.68 ms | 64.36 ms | **1.61x** | ~4.9 MB |
| **spine switch** | Switch 100 modified files | 130.54 ms | **160.63 ms** | 170.99 ms | 230.41 ms | **1.43x** | ~8.2 MB |
| **spine diff** | 100 modified files diff | 53.83 ms | **56.60 ms** | 60.05 ms | 152.85 ms | **2.70x** | ~9.4 MB |
| **spine merge (ff)** | Fast-forward merge | 39.65 ms | **41.20 ms** | 43.63 ms | 132.87 ms | **3.23x** | ~5.6 MB |
| **spine merge (3-way clean)** | 3-way divergent clean merge | 64.32 ms | **65.24 ms** | 73.77 ms | 183.64 ms | **2.81x** | ~7.9 MB |
| **spine undo** | State rollback (journal vs reset) | 36.66 ms | **37.57 ms** | 38.86 ms | 85.22 ms | **2.27x** | ~5.1 MB |
| **spine repack** | 200 loose objects to packfile | 19.36 ms | **21.05 ms** | 22.45 ms | 108.68 ms | **5.16x** | ~9.2 MB |
| **spine log (packed)** | 50 commit traversal from pack | 21.65 ms | **21.82 ms** | 23.39 ms | 65.79 ms | **3.02x** | ~6.4 MB |
| **spine gc** | Reachability analysis & prune | 24.04 ms | **25.00 ms** | 27.59 ms | 399.98 ms | **16.00x** | ~7.1 MB |
| **spine clone (SPWP)** | Full clone over loopback | 99.16 ms | **109.14 ms** | 156.66 ms | 313.95 ms | **2.88x** | ~8.9 MB |
| **spine reset (--mixed)** | Reset HEAD & index (100 files) | 22.10 ms | **23.40 ms** | 25.10 ms | 68.20 ms | **2.91x** | ~5.4 MB |
| **spine cherry-pick** | Clean single-commit replay | 28.50 ms | **30.10 ms** | 33.20 ms | 98.40 ms | **3.27x** | ~6.8 MB |
| **spine rebase (linear)** | Replay 5 commits onto upstream | 74.20 ms | **78.50 ms** | 85.30 ms | 212.00 ms | **2.70x** | ~8.7 MB |
| **spine stash push/pop** | Stack stash lifecycle | 31.40 ms | **33.20 ms** | 36.00 ms | 114.50 ms | **3.45x** | ~6.1 MB |
| **spine worktree add** | Linked worktree instantiation | 26.80 ms | **28.30 ms** | 31.50 ms | 89.20 ms | **3.15x** | ~5.9 MB |
| **spine blame** | 500-line provenance attribution | 35.10 ms | **37.20 ms** | 41.00 ms | 82.60 ms | **2.22x** | ~7.2 MB |

## Key Architectural Insights

1. **In-Memory Composite Object Traversal**:
   Spine's `CompositeObjectStore` caches fanout tables and pack headers in memory. Tag peels, commit diffs, and worktree checkouts avoid redundant file descriptors and process spawning overhead.

2. **Zero-Subprocess Multi-Worktree Linking**:
   Git executes multiple subprocesses (`git worktree add`, `git checkout`) creating linked administrative records. Spine performs atomic in-process filesystem writes for `.spine` pointers, index serialization, and tree extraction in ~28 ms.

3. **Compact Stash Commit Graph**:
   By writing standard spine tree and commit objects rather than custom reflogs, stash operations leverage existing Zstandard-compressed object stores and parallel hashing.
