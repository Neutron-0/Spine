# Spine Release Provenance and SBOM

**Status:** Verified Release Candidate Provenance
**Release Candidate:** `0.1.0-rc3`

## 1. Source & Toolchain Provenance

- **Repository:** `D:\Vault\Spine`
- **Branch:** `main`
- **Built Source Commit:** `90a695d940efcc3e4a2f3f4d856b416ac707d1f1` (source code, tests, and `docs/sbom.json` compiled into all release artifacts below; this provenance document commit is a `docs/`-only commit immediately following build verification)
- **Built Source Tree Hash:** `3140d1fe19ca069226ffdbcb3472002097153bdc`
- **Build Completion Timestamp (UTC):** `2026-11-12T05:51:16Z`
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

All four production executables were built from source commit `90a695d940efcc3e4a2f3f4d856b416ac707d1f1`, staged under `target/release-candidate/`, and recorded in `target/release-candidate/SHA256SUMS`:

| Binary | Size (Bytes) | SHA-256 |
|---|---|---|
| `spine.exe` | `8864768` | `4c9c69b0a593e88f6c127a2e68757f23ea03a2392d8f7d6adbb7c069ca7cec34` |
| `spine-coordinator.exe` | `2694144` | `0b8f086fd8eb6ba39e8db679bcfa2eae3abb6fdde7e341e0e7d8dc8466503b02` |
| `spine-storage-node.exe` | `2449408` | `279f287598987e00bdea4e4e7f1fb1a34d8c6c663387209e422ebf11940c62e5` |
| `spine-api.exe` | `22696960` | `8bc9143e9610012d48b7dad83105c91dd77c783398838abb9e48fda88e782412` |

Checksums manifest (`target/release-candidate/SHA256SUMS`):

```text
4c9c69b0a593e88f6c127a2e68757f23ea03a2392d8f7d6adbb7c069ca7cec34  spine.exe
0b8f086fd8eb6ba39e8db679bcfa2eae3abb6fdde7e341e0e7d8dc8466503b02  spine-coordinator.exe
279f287598987e00bdea4e4e7f1fb1a34d8c6c663387209e422ebf11940c62e5  spine-storage-node.exe
8bc9143e9610012d48b7dad83105c91dd77c783398838abb9e48fda88e782412  spine-api.exe
```

## 5. CycloneDX Software Bill of Materials (SBOM)

- **Format:** CycloneDX `1.5` JSON (`bomFormat: CycloneDX`, `specVersion: 1.5`)
- **Path:** `docs/sbom.json`
- **SBOM SHA-256:** `6a86f445715d895dad11592d52fe5e3cbed5f5d34a40add256414db292c94a1f`
- **SBOM Timestamp (UTC):** `2026-11-12T05:45:22.089277+00:00`
- **Root Component:** `pkg:cargo/spine-platform@0.1.0-rc3` (`version: 0.1.0-rc3`)
- **Total Unique Crate Components:** `465` (verified exact set equality against `git-alternative/Cargo.lock` + `github-alternative/backend/Cargo.lock`, with `0` missing or extra packages)
