# Spine Release Provenance and SBOM

**Status:** Verified Release Candidate Provenance
**Release Candidate:** `0.1.0-rc3`

## 1. Source & Toolchain Provenance

- **Repository:** `D:\Vault\Spine`
- **Branch:** `main`
- **Built Source Commit:** `3eb5e23294f116b67609f47c61e569eb76fa0707` (source code, migrations, tests, and `docs/sbom.json` compiled into all release artifacts below; this provenance document commit is a `docs/`-only commit immediately following build verification)
- **Built Source Tree Hash:** `bec49d6d9ad6b312bf69d32b82d6e729e03369b8`
- **Build Completion Timestamp (UTC):** `2026-11-12T08:57:21Z`
- **Host Target:** `x86_64-pc-windows-msvc`
- **Compiler (`rustc --version`):** `rustc 1.97.1 (8bab26f4f 2026-07-14)` (commit `8bab26f4f68e0e26f0bb7960be334d5b520ea452`, LLVM `22.1.6`)
- **Cargo (`cargo --version`):** `cargo 1.97.1 (c980f4866 2026-06-30)`

## 2. Build Commands & Profile

- **Profile:** `--release` (`release` profile, optimized)
- **Native VCS & Cluster Workspace:**
  `cargo build --release -j 1 --manifest-path git-alternative/Cargo.toml`
- **Backend API Server:**
  `cargo build --release -j 2 --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml`

## 3. Reproducibility & Lockfile Digests

Both workspaces build with tracked `Cargo.lock` dependency graphs:

- `git-alternative/Cargo.lock` (SHA-256):
  `f0be820cc613a6359eb8fa4d68f05a44831fb4c9a4561099be6edb4c33531fdf`
- `github-alternative/backend/Cargo.lock` (SHA-256):
  `4f15df88dfeede2d5583d9e73b78a3035f1ac7f96a2f2caa3a3c7c8e3af8feae`

## 4. Release Candidate Artifact Inventory & SHA-256 Checksums

All four production executables were built from source commit `3eb5e23294f116b67609f47c61e569eb76fa0707`, staged under `target/release-candidate/`, and recorded in `target/release-candidate/SHA256SUMS`:

| Binary | Size (Bytes) | SHA-256 |
|---|---|---|
| `spine.exe` | `8892416` | `65ea4bf118113722fce876211e8ff2028b5f4273813d349c08374f03995104ab` |
| `spine-coordinator.exe` | `3041792` | `66ccd6ac73c0154f48b84409d425801f5c314a6733def7609340f6a84696d06f` |
| `spine-storage-node.exe` | `2766848` | `7d969321f868417483b952f36d26c68a8a4ac0b6e2a7308bf566224e02eb89cf` |
| `spine-api.exe` | `22773248` | `005f58f0d541e27f5cdc8daf723a3d8ab538345b3c5e72b66c889a1596026f7d` |

Checksums manifest (`target/release-candidate/SHA256SUMS`):

```text
65ea4bf118113722fce876211e8ff2028b5f4273813d349c08374f03995104ab  spine.exe
66ccd6ac73c0154f48b84409d425801f5c314a6733def7609340f6a84696d06f  spine-coordinator.exe
7d969321f868417483b952f36d26c68a8a4ac0b6e2a7308bf566224e02eb89cf  spine-storage-node.exe
005f58f0d541e27f5cdc8daf723a3d8ab538345b3c5e72b66c889a1596026f7d  spine-api.exe
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
  - **Protocol Rationale:** `ClusterRpcRequest` and `ClusterRpcResponse` serialize via `serde_json`, where `payload_bytes: Vec<u8>` encodes as a decimal integer array with `4x` worst-case expansion (`[255,...]`). A `15 MiB` payload fits within `MAX_CLUSTER_RPC_FRAME_BYTES` (`64 MiB`) with `>4 MiB` headroom for `VolumeSnapshotManifest` and protocol fields.
  - **Bounded Capture Enforcement:** `BoundedVecWriter` aborts `capture_volume_snapshot` and coordinator `capture_snapshot` immediately when compressed/serialized bytes would exceed `15 MiB`, preventing unbounded memory allocation and cleanly releasing maintenance fencing on abort.
  - **Pre-Restore Sizing Check:** Before scheduling a production restore window, operators must verify each remote participant's compressed snapshot (`tar -czf - repos chunks manifests | wc -c` for storage nodes; `coordinator_state.json` size for coordinator) is `<= 15,728,640` bytes, or use offline/out-of-band volume replication for larger repositories.
