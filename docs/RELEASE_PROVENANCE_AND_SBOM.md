# Spine Release Provenance and SBOM

**Status:** Verified Release Candidate Provenance
**Release Candidate:** `0.1.0-rc3`

## 1. Source & Toolchain Provenance

- **Repository:** `D:\Vault\Spine`
- **Branch:** `main`
- **Built Source Commit:** `3fcded082374542b28bebff895476f6639eb29d3` (source code, migrations, tests, and `docs/sbom.json` compiled into all release artifacts below; this provenance document commit is a `docs/`-only commit immediately following build verification)
- **Built Source Tree Hash:** `34673c35c12981ad6e96621cd19204e42dbf2c05`
- **Build Completion Timestamp (UTC):** `2026-11-12T08:01:27Z`
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

All four production executables were built from source commit `3fcded082374542b28bebff895476f6639eb29d3`, staged under `target/release-candidate/`, and recorded in `target/release-candidate/SHA256SUMS`:

| Binary | Size (Bytes) | SHA-256 |
|---|---|---|
| `spine.exe` | `8864768` | `f112ee033f4cee70d900c40eb6d900010a44623d5298e55ca946b6134e3a1ec0` |
| `spine-coordinator.exe` | `2692096` | `503be6aea55364f9fb4935d3d61e16145e1c6fc49eb47906c43391de95cb6224` |
| `spine-storage-node.exe` | `2450944` | `bfce3912bb9b53666afbde1c61248b22a153b6a4036c098dc990ae39bbe4288c` |
| `spine-api.exe` | `22695936` | `093291139b07cc8760705f0febd637fdfad8b64325a96d9d4be4bacdb348e353` |

Checksums manifest (`target/release-candidate/SHA256SUMS`):

```text
f112ee033f4cee70d900c40eb6d900010a44623d5298e55ca946b6134e3a1ec0  spine.exe
503be6aea55364f9fb4935d3d61e16145e1c6fc49eb47906c43391de95cb6224  spine-coordinator.exe
bfce3912bb9b53666afbde1c61248b22a153b6a4036c098dc990ae39bbe4288c  spine-storage-node.exe
093291139b07cc8760705f0febd637fdfad8b64325a96d9d4be4bacdb348e353  spine-api.exe
```

## 5. CycloneDX Software Bill of Materials (SBOM)

- **Format:** CycloneDX `1.5` JSON (`bomFormat: CycloneDX`, `specVersion: 1.5`)
- **Path:** `docs/sbom.json`
- **SBOM SHA-256:** `6a86f445715d895dad11592d52fe5e3cbed5f5d34a40add256414db292c94a1f`
- **SBOM Timestamp (UTC):** `2026-11-12T05:45:22.089277+00:00`
- **Root Component:** `pkg:cargo/spine-platform@0.1.0-rc3` (`version: 0.1.0-rc3`)
- **Total Unique Crate Components:** `465` (verified exact set equality against `git-alternative/Cargo.lock` + `github-alternative/backend/Cargo.lock`, with `0` missing or extra packages)
