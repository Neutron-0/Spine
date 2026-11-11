# SPINE RELEASE PROVENANCE AND SOFTWARE BILL OF MATERIALS (SBOM)
## Batch 21N Full-System Stabilization and Release Candidate Verification

This document establishes the build provenance, reproducibility manifest, and cryptographic chain-of-custody for the Spine Release Candidate binaries produced under the **Batch 21N Master Stabilization Campaign**.

---

## 1. Build Environment and Provenance

- **Operating System:** Windows 11 (NT 10.0.26200 x86_64)
- **Rust Toolchain:** `rustc 1.97.1 (8bab26f4f 2026-07-14)`
- **Cargo Version:** `cargo 1.97.1 (c980f4866 2026-06-30)`
- **Host Target Triple:** `x86_64-pc-windows-msvc`
- **LLVM Version:** `22.1.6`
- **Base Commit Hash:** `f496e5e819f02a2e6737b1c33f325aac8338574b`
- **Branch:** `main`
- **Build Timestamp (UTC):** `2026-11-11T16:37:00Z`
- **Compilation Mode:** Full release optimization (`--release`), LTO enabled in production profile.

---

## 2. Lockfile Digests

The dependency graph for all components is locked and verified with the following SHA-256 digests:

| Lockfile Path | SHA-256 Digest |
| :--- | :--- |
| `git-alternative/Cargo.lock` | `f0be820cc613a6359eb8fa4d68f05a44831fb4c9a4561099be6edb4c33531fdf` |
| `github-alternative/backend/Cargo.lock` | `4f15df88dfeede2d5583d9e73b78a3035f1ac7f96a2f2caa3a3c7c8e3af8feae` |

---

## 3. Build Invocations

All candidate binaries were produced cleanly using the following invocations:

```powershell
# 1. Native Git alternative and cluster daemon binaries
cargo build --release --manifest-path git-alternative/Cargo.toml -j1

# 2. REST API / Backup Coordinator binary
cargo build --release --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml -j2
```

---

## 4. Release Candidate Artifact Manifest (`target/release-candidate/SHA256SUMS`)

All binaries were staged into `target/release-candidate/` and authenticated via SHA-256:

| Binary | Size (Bytes) | SHA-256 Checksum | Description |
| :--- | :--- | :--- | :--- |
| `spine.exe` | 8,829,440 | `3f83d398ca2ed473e081b65e5c23f8d8b1b894eb8a656fc99a15f4835321f17f` | Native CLI client and repository management |
| `spine-coordinator.exe` | 2,654,720 | `3b6a3b948dd5987aebabcd370698f056e23edac1a4dc94113fef2615f9363e16` | Distributed cluster consensus and routing coordinator |
| `spine-storage-node.exe` | 2,441,728 | `4c27906172fb63b9fe9ef0c68feaec9ec4e673a12d7be4837dfb2f7be91835e6` | Storage-node volume daemon with fenced write-guards |
| `spine-api.exe` | 22,324,736 | `a28f828430ead4b8459b6e3ffa9fa0e84a260670430aeb5c08719da6afe0d840` | Production Web API, Auth, and Backup/Restore orchestrator |

---

## 5. Security & Verification Assertions

1. **Selector Authority:** Single source of database selection truth (`_spine_active_database_selector` in maintenance database) via transactional row-locking (`SELECT FOR UPDATE`) CAS updates. Filesystem selector cache removed from write gates and readback validation.
2. **Participant Integrity:** Pre-decision revalidation verifies extracted content hashes and counts directly on disk before committing restore state transitions.
3. **Fail-Closed Write Gating:** Database readiness, write-permit acquisition, and HTTP route pipelines reject mutations unless authoritative database generation matches storage generation and active journal state is clear.
4. **Win32 Durability:** All journal, state, and selector atomic replacements use native Win32 `ReplaceFileW` and `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` (or POSIX `rename` + parent directory `sync_all`), completely resolving Windows error 650.
5. **Decryption Confidentiality:** Zero unencrypted intermediate archives written to disk during backup decompression; stream decryption executes chunk-by-chunk in bounded memory via AES-256-GCM.
