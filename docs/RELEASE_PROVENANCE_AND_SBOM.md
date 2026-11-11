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
- **Build Timestamp (UTC):** `2026-11-11T19:34:00Z`
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
| `spine.exe` | 8,862,720 | `85ac6be2ae63aad5dbf2f874f628b411b252b9a3e3f42e203ad4f5f5638ce778` | Native CLI client and repository management |
| `spine-coordinator.exe` | 2,691,584 | `9d17a03dd1d481f2e6a41162c322de5488d4ce2609316dac9c248404171a0b34` | Distributed cluster consensus and routing coordinator |
| `spine-storage-node.exe` | 2,448,384 | `d59b35c11323c206e542f18de8e25227dd7d735af746431ae370e8d02609859a` | Storage-node volume daemon with fenced write-guards |
| `spine-api.exe` | 22,470,144 | `9a0b5482adf248b1afdba21439099c0b4c69a1977762c00ddf36ce54d1adf0e9` | Production Web API, Auth, and Backup/Restore orchestrator |

---

## 5. Machine-Readable CycloneDX SBOM

A complete, machine-readable Software Bill of Materials conforming to the [CycloneDX v1.5 specification](http://cyclonedx.org/schema/bom-1.5.json) is published alongside this document at:

- **JSON Document:** `docs/sbom.json`
- **Component Count:** 465 verified dependencies with package URLs (purl) and SHA-256 package digests
- **Format:** CycloneDX JSON 1.5

---

## 6. Security & Verification Assertions

1. **Selector Authority:** Single source of database selection truth (`_spine_active_database_selector` in maintenance database) via transactional row-locking (`SELECT FOR UPDATE`) CAS updates. Filesystem selector cache removed from write gates and readback validation.
2. **Post-Decision Invariant:** Zero participant rollbacks permitted after `CommitDecision` is durably persisted. Failed post-decision transitions fail closed with write fencing preserved, and restart recovery converges forward idempotently.
3. **Seven-Participant Binding:** Pre-decision revalidation verifies all 7 participants (`postgres_data`, `spine_storage`, `storage_1_data`, `storage_2_data`, `storage_3_data`, `coordinator_data`, `spine_ssh`) with exact field-level comparison of `restore_id`, `backup_id`, `target_generation`, and payload SHA-256 digests.
4. **Fail-Closed Write Gating:** Database readiness, write-permit acquisition, and HTTP route pipelines reject mutations unless authoritative database generation matches storage generation and active journal state is clear.
5. **Win32 Durability:** All journal, state, and selector atomic replacements use native Win32 `ReplaceFileW` and `MoveFileExW` with `MOVEFILE_WRITE_THROUGH` (or POSIX `rename` + parent directory `sync_all`), completely resolving Windows error 650.
6. **Decryption Confidentiality:** Zero unencrypted intermediate archives written to disk during backup decompression; stream decryption executes chunk-by-chunk in bounded memory via AES-256-GCM.
