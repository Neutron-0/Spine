# PERFORMANCE BASELINES & RESOURCE LIMITS

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. NATIVE VCS BENCHMARK RESULTS

Measurements executed directly via `benchmark_runner.exe` on Windows 11 MSVC (NTFS, x86_64, 1 warm-up cycle, 5 measured iterations per operation):

| Operation | Workload Description | Spine Min | Spine Median | Spine P95 | Official Git Median | Speedup Ratio | Memory Footprint |
|---|---|---|---|---|---|---|---|
| **`spine init`** | Warm repository initialization | 22.36 ms | **24.06 ms** | 25.32 ms | 89.21 ms | **3.71x** | ~4,200 KB |
| **`spine branch create`** | Atomic branch ref creation | 42.66 ms | **46.26 ms** | 51.55 ms | 65.42 ms | **1.41x** | ~4,900 KB |
| **`spine switch`** | Switch branch with 100 modified files | 154.72 ms | **160.77 ms** | 165.73 ms | 199.56 ms | **1.24x** | ~8,200 KB |
| **`spine diff`** | 100 modified files full diff | 55.43 ms | **64.68 ms** | 69.34 ms | 165.80 ms | **2.56x** | ~9,400 KB |
| **`spine merge (ff)`** | Fast-forward branch merge | 47.13 ms | **50.11 ms** | 53.13 ms | 106.34 ms | **2.12x** | ~5,600 KB |
| **`spine merge (3-way clean)`** | 3-way divergent clean merge | 72.42 ms | **76.57 ms** | 84.02 ms | 156.72 ms | **2.05x** | ~7,900 KB |
| **`spine undo`** | State rollback (journal vs reset) | 42.83 ms | **47.22 ms** | 51.22 ms | 75.67 ms | **1.60x** | ~5,100 KB |
| **`spine repack`** | Consolidate 200 loose objects into pack + fanout index | 22.35 ms | **22.44 ms** | 25.19 ms | 107.69 ms | **4.80x** | ~9,200 KB |
| **`spine log (packed)`** | Traverse 50 commits from `.spk` packfile | 22.33 ms | **24.62 ms** | 26.29 ms | 66.29 ms | **2.69x** | ~6,400 KB |
| **`spine gc`** | Reachability analysis and loose object pruning | 25.01 ms | **28.47 ms** | 29.87 ms | 341.66 ms | **12.00x** | ~7,100 KB |
| **`spine clone (SPWP)`** | Full clone over loopback wire protocol | 104.82 ms | **117.87 ms** | 134.41 ms | 283.95 ms | **2.41x** | ~8,900 KB |

---

## 2. FORGE BACKEND OPERATION LATENCIES

Measured across 100 sequential requests using the in-memory test harness:

| Operation | Median Latency | P95 Latency | Concurrency Bound |
|---|---|---|---|
| **User Registration (Argon2id)** | 32.1 ms | 48.6 ms | Rate limited to 10 req/min |
| **Session Authentication / Login** | 31.4 ms | 46.2 ms | Rate limited to 20 req/min |
| **Repository Metadata Fetch** | 0.8 ms | 1.4 ms | Unbounded |
| **Global Search Query** | 1.2 ms | 2.5 ms | Capped at 50 results |
| **Code Search Snapshot Indexing** | 4.6 ms | 8.2 ms | Max 10,000 files / 1 MB per file |
| **Webhook Event Enqueue (Outbox)** | 0.4 ms | 0.9 ms | Transactional |
| **Secret Scan (100 files)** | 3.8 ms | 6.5 ms | Max 100,000 lines |
| **SBOM Generation (SPDX / CycloneDX)** | 2.1 ms | 3.9 ms | Max 5,000 dependencies |

---

## 3. CONFIGURED RESOURCE LIMITS

1. **SPWP Wire Frames:** Maximum frame size 64 MiB; frames exceeding this limit trigger immediate connection termination.
2. **Webhook Ingestion:** Response payloads capped at 64 KiB.
3. **Artifact Upload Quota:** Configurable per repository (default: 5 GB for standard repositories, 50 GB for enterprise).
4. **Workflow Execution Timeout:** Step timeout defaulted to 60 seconds (max 3,600s).
5. **Database Connection Pool:** Capped at 50 connections with 5-second connection acquisition timeout.
