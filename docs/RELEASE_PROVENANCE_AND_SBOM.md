# Spine Release Provenance and SBOM

**Status:** Verified Release Candidate Provenance
**Release Candidate:** `0.1.0-rc3`

## 1. Source & Toolchain Provenance

- **Repository:** `D:\Vault\Spine`
- **Branch:** `main`
- **Built Source Commit:** `c0f4c7bc4950183aff1d6a4dd28745c1153ac57c` (source code, migrations, tests, and `docs/sbom.json` compiled into all release artifacts below; this provenance document commit is a `docs/`-only commit immediately following build verification)
- **Built Source Tree Hash:** `4ce07fd8d70b1f0b73608278b031dcdb0b6ece9f`
- **Build Start Timestamp (UTC):** `2026-11-13T10:32:12Z`
- **Build Completion Timestamp (UTC):** `2026-11-13T10:55:42Z`
- **Artifact Staging Timestamp (UTC):** `2026-11-13T10:56:00Z`
- **Host Target:** `x86_64-pc-windows-msvc`
- **Compiler (`rustc --version`):** `rustc 1.97.1 (8bab26f4f 2026-07-14)` (commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, LLVM `22.1.6`)
- **Cargo (`cargo --version`):** `cargo 1.97.1 (c980f4866 2026-06-30)`

## 2. Build Commands & Profile

- **Profile:** `--release` (`release` profile, optimized)
- **Native VCS & Cluster Workspace:**
  `cargo build --release -j 1 --manifest-path git-alternative/Cargo.toml`
  - Output paths:
    - `d:\Vault\Spine\git-alternative\target\release\spine.exe`
    - `d:\Vault\Spine\git-alternative\target\release\spine-coordinator.exe`
    - `d:\Vault\Spine\git-alternative\target\release\spine-storage-node.exe`
  - Exit code: `0`
- **Backend API Server:**
  `cargo build --release -j 1 --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml`
  - Output path: `d:\Vault\Spine\github-alternative\backend\target\release\spine-api.exe`
  - Build duration: 22m 11s (compiled from clean dependency tree)
  - Exit code: `0`

## 3. Reproducibility & Lockfile Digests

Both workspaces build with tracked `Cargo.lock` dependency graphs:

- `git-alternative/Cargo.lock` (SHA-256):
  `f0be820cc613a6359eb8fa4d68f05a44831fb4c9a4561099be6edb4c33531fdf`
- `github-alternative/backend/Cargo.lock` (SHA-256):
  `4f15df88dfeede2d5583d9e73b78a3035f1ac7f96a2f2caa3a3c7c8e3af8feae`

## 4. Release Candidate Artifact Inventory & SHA-256 Checksums

All four production executables were built from source commit `c0f4c7bc4950183aff1d6a4dd28745c1153ac57c`, staged under `target/release-candidate/`, and recorded in `target/release-candidate/SHA256SUMS`:

| Binary | Size (Bytes) | Staged Modification Time (UTC) | SHA-256 |
|---|---|---|---|
| `spine.exe` | `8892416` | `2026-11-13 10:56:00 UTC` | `d7ed0eb73fa237e15bf9b5d85fa1f475f5a76554a7a735aa45a9fe13f754a3e6` |
| `spine-coordinator.exe` | `3041792` | `2026-11-13 10:56:00 UTC` | `2f318c2960f30877328ee8bf451240c334b961131621835bb9a8213fdd51d8be` |
| `spine-storage-node.exe` | `2766848` | `2026-11-13 10:56:00 UTC` | `36902910c703991d1ff6f65d2ae17d9b1f55285dbad1bfbd18ff028d4c98eccf` |
| `spine-api.exe` | `23265280` | `2026-11-13 10:56:00 UTC` | `b7880b8b91a245175f4fe97f1ed0096c100ffc32daa25ce51ca833f359a35d77` |

Checksums manifest (`target/release-candidate/SHA256SUMS`):

```text
d7ed0eb73fa237e15bf9b5d85fa1f475f5a76554a7a735aa45a9fe13f754a3e6  spine.exe
2f318c2960f30877328ee8bf451240c334b961131621835bb9a8213fdd51d8be  spine-coordinator.exe
36902910c703991d1ff6f65d2ae17d9b1f55285dbad1bfbd18ff028d4c98eccf  spine-storage-node.exe
b7880b8b91a245175f4fe97f1ed0096c100ffc32daa25ce51ca833f359a35d77  spine-api.exe
```

## 5. CycloneDX Software Bill of Materials (SBOM)

- **Format:** CycloneDX `1.5` JSON (`bomFormat: CycloneDX`, `specVersion: 1.5`)
- **Path:** `docs/sbom.json`
- **SBOM SHA-256:** `6a86f445715d895dad11592d52fe5e3cbed5f5d34a40add256414db292c94a1f`
- **SBOM Timestamp (UTC):** `2026-11-12T05:45:22.089277+00:00`
- **Root Component:** `pkg:cargo/spine-platform@0.1.0-rc3` (`version: 0.1.0-rc3`)
- **Total Unique Crate Components:** `465` (verified exact set equality against `git-alternative/Cargo.lock` + `github-alternative/backend/Cargo.lock`, with `0` missing or extra packages)

## 6. Supported Operating Limits & Remote Snapshot Sizing

- **Remote Participant RPC Snapshot Limit:** `15 MiB` (`15,728,640` bytes) compressed payload (`MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES`) per coordinator or storage-node participant transferred over single-frame `ClusterRpcRequest` / `ClusterRpcResponse`.
- **Validation Status:** **UNVALIDATED FOR PRODUCTION-SIZED REPOSITORIES.** The 15 MiB limit has been validated strictly for protocol framing safety, bounded-allocation aborts, and test fixtures (<100 KiB). It has NOT been validated against representative multi-gigabyte production repositories.
- **Protocol Framing Constraint:** `ClusterRpcRequest` and `ClusterRpcResponse` serialize via `serde_json`, where `payload_bytes: Vec<u8>` encodes as an ASCII decimal integer array with `4x` worst-case expansion (`[255,...]`). A `15 MiB` compressed payload expands to ~60 MiB in JSON, leaving ~4 MiB headroom below `MAX_CLUSTER_RPC_FRAME_BYTES` (`64 MiB`).
- **Bounded Capture Enforcement:** `BoundedVecWriter` aborts `capture_volume_snapshot` and coordinator `capture_snapshot` immediately when compressed/serialized bytes exceed `15 MiB`, preventing unbounded memory allocation and releasing maintenance fencing on abort.
- **Mandatory Deployment Prerequisite:** Before scheduling or enabling automated cluster backups in any production deployment, operators must independently verify that each storage node's compressed repository payload (`tar -czf - repos chunks manifests | wc -c`) and coordinator state file size are `<= 15,728,640` bytes. If repository volumes exceed 15 MiB, operators must employ out-of-band volume snapshotting (e.g. EBS / ZFS volume replication) or upgrade the cluster transport to multi-frame chunked streaming.
