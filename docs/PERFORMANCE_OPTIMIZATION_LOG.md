# PERFORMANCE OPTIMIZATION LOG

## Entry 01: Hash Engine Selection (BLAKE3 vs SHA-256)
- **Hypothesis:** BLAKE3 achieves 4x–8x throughput over SHA-256 due to tree hashing and AVX2/AVX-512 SIMD vectorization.
- **Verification:** Benchmarked on 1KB, 64KB, and 1MB byte payloads. BLAKE3 sustained ~3.8 GB/s on x86_64, whereas SHA-256 peaked at ~480 MB/s.
- **Decision:** Adopted BLAKE3 as the default object identifier hash algorithm; retained SHA-256 as configurable compatibility mode.

## Entry 02: Compression Algorithm (Zstandard vs zlib Deflate)
- **Hypothesis:** Zstandard level 3 provides superior compression ratios while decompressing at >1.2 GB/s, drastically improving commit read latency.
- **Verification:** On a 10MB binary payload, Spine ingested and compressed in 37.32 ms, compared to Git's 170.75 ms (4.58x speedup).
- **Decision:** Hardcoded Zstandard for loose object storage.

## Entry 03: Bottom-Up Tree Construction Optimization
- **Problem:** When staging deeply nested directories, ancestor directories could be processed before child subtrees, resulting in missing subtrees.
- **Fix:** Switched tree directory processing to sort directories by descending path length (`dirs.sort_by_key(|b| std::cmp::Reverse(b.len()))`). Guarantees that deepest child subtrees are written and hashed first, allowing parent directories to embed their computed `ObjectId`s with zero redundant passes.

## Entry 04: Temporary File POSIX Semantics on Windows
- **Problem:** Windows file locks can cause concurrent rename failures if a target object file already exists.
- **Mitigation:** If `fs::rename` fails and the target object file exists on disk, Spine checks that the target exists and removes the temporary file, ensuring atomic idempotency.

## Entry 05: Benchmark Runner Warm-Up Calibration on Windows NTFS
- **Problem:** In Batch 01, `spine init` cold start was measured at 729 ms due to cold process initialization and Windows Defender DLL/file scanning on first run.
- **Verification:** Adding 1 warm-up cycle and sampling median/p95 over 5 iterations revealed true warm `spine init` performance is **20.30 ms** (vs Git's 101.93 ms, achieving a **5.02x speedup**).
- **Decision:** All future benchmark measurements must execute a mandatory warm-up cycle and report min, median, p95, and max.

## Entry 06: Zero-Copy In-Memory Three-Way Merge Reconciliation
- **Hypothesis:** Reconciling three trees in memory (`BTreeMap<String, (FileMode, ObjectId)>`) and checking for identity shortcuts before reading object payloads eliminates >80% of disk I/O on typical merges.
- **Verification:** Benchmarking a 3-way divergent merge resulted in 65.51 ms median execution for Spine vs 159.62 ms for Git (a **2.44x speedup**).
- **Decision:** Integrated in-memory path classification with 13 discrete reconciliation cases in `spine-merge`.
