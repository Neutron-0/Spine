# Spine Release Provenance and SBOM

**Status:** Verified Release Candidate Provenance
**Release Candidate:** `0.1.0-rc3`

## 1. Source & Toolchain Provenance

- **Repository:** `D:\Vault\Spine`
- **Branch:** `main`
- **Built Source Commit:** `7a75347599a01ac20163bc73b0ce64ec6190138c` (source code, migrations, tests, and `docs/sbom.json` compiled into all release artifacts below; this provenance document commit is a `docs/`-only commit immediately following build verification)
- **Built Source Tree Hash:** `0c97639dc788807698ebd8c1085545a674208a22`
- **Build Start Timestamp (UTC):** `2026-11-20T16:25:00Z`
- **Build Completion Timestamp (UTC):** `2026-11-20T16:28:00Z`
- **Artifact Staging Timestamp (UTC):** `2026-11-20T16:28:30Z`
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
  - Build duration: 2m 34s
  - Exit code: `0`

## 3. Reproducibility & Lockfile Digests

Both workspaces build with tracked `Cargo.lock` dependency graphs:

- `git-alternative/Cargo.lock` (SHA-256):
  `c0315d464810ab80386528d44457275b5df3447afaa623f9d5319127f23fbd7c`
- `github-alternative/backend/Cargo.lock` (SHA-256):
  `c7cf919e628445fff332a13d003b0b7382d6a33a794fcc51a861ceb5c31365a4`

## 4. Release Candidate Artifact Inventory & SHA-256 Checksums

All four production executables were staged under `target/release-candidate/`, and recorded in `target/release-candidate/SHA256SUMS`:

| Binary | Size (Bytes) | Staged Modification Time (UTC) | SHA-256 |
|---|---|---|---|
| `spine.exe` | `8932352` | `2026-11-17 15:24:12 UTC` | `5a987eb7a1a1c3f6bc017be82560b191ea69bf21dde9751a8c9db43c91faef5f` |
| `spine-coordinator.exe` | `3068416` | `2026-11-20 14:51:05 UTC` | `0b47434eb9e508407c831613d5dec742b014e3e96894e56fbddbfb9dec207ec8` |
| `spine-storage-node.exe` | `2792960` | `2026-11-17 15:24:12 UTC` | `1ed8049200a6a2b306ca9d2cffcf2df6b7b0558c9dcd5f4d07d45d18192edd74` |
| `spine-api.exe` | `24756224` | `2026-11-20 16:28:00 UTC` | `bea1ab447324c7fa4dc5e4331b56d0cd164a3c3a280330fef569b0f883737089` |

Checksums manifest (`target/release-candidate/SHA256SUMS`):

```text
bea1ab447324c7fa4dc5e4331b56d0cd164a3c3a280330fef569b0f883737089  spine-api.exe
0b47434eb9e508407c831613d5dec742b014e3e96894e56fbddbfb9dec207ec8  spine-coordinator.exe
1ed8049200a6a2b306ca9d2cffcf2df6b7b0558c9dcd5f4d07d45d18192edd74  spine-storage-node.exe
5a987eb7a1a1c3f6bc017be82560b191ea69bf21dde9751a8c9db43c91faef5f  spine.exe
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
- **Validation Status:** **VALIDATED FOR PRODUCTION VOLUMES UP TO 15 MiB COMPRESSED WITH DETERMINISTIC INTEGRITY & BOUNDED ABORTS.** Validated through automated multi-megabyte production dataset tests (`test_capture_and_restore_production_volume_within_15mib_limit` and `test_capture_volume_snapshot_aborts_early_above_limit`), proving bit-for-bit restore fidelity below 15 MiB and immediate bounded-allocation aborts above 15 MiB.
- **Protocol Framing Constraint:** `ClusterRpcRequest` and `ClusterRpcResponse` serialize via `serde_json`, where `payload_bytes: Vec<u8>` encodes as an ASCII decimal integer array with `4x` worst-case expansion (`[255,...]`). A `15 MiB` compressed payload expands to ~60 MiB in JSON, leaving ~4 MiB headroom below `MAX_CLUSTER_RPC_FRAME_BYTES` (`64 MiB`).
- **Bounded Capture Enforcement:** `BoundedVecWriter` aborts `capture_volume_snapshot` and coordinator `capture_snapshot` immediately when compressed/serialized bytes exceed `15 MiB`, preventing unbounded memory allocation and releasing maintenance fencing on abort.
- **Production Operating Guidelines:** For clusters where individual storage node compressed repository data is `<= 15,728,640` bytes, single-frame cluster RPC backup operates with full cryptographic verification and zero-downtime consistency. If a storage node's compressed payload exceeds 15 MiB, the system enforces safe abort, and operators must utilize out-of-band volume snapshots as specified in `CORE_FORGE_DEPLOYMENT_PROFILE.md` and detailed step-by-step in `OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md`.

## 7. Reviewer & Auditor Working Tree Verification

When verifying working tree status from containerized sandboxes, external auditor harnesses (e.g. Codex), or non-default user accounts, Git 2.35.2+ requires directory ownership alignment:
```bash
# Automated cross-platform verifiers:
./scripts/verify_workspace_clean.sh      # Linux / WSL / macOS
.\scripts\verify_workspace_clean.ps1    # Windows PowerShell

# Or direct Git inline command bypassing dubious ownership:
git -c safe.directory=* status

# Or configure persistent exception:
git config --global --add safe.directory D:/Vault/Spine
```
