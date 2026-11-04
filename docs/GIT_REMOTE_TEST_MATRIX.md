# Spine Forge — Git Remote & LFS Verification Matrix

## 1. Test Environment

- **Host OS:** Windows 11 Enterprise (AMD64)
- **Rust Toolchain:** rustc 1.84+ / Cargo
- **Git Binary Oracle:** `git version 2.53.0.windows.1`
- **Git LFS Binary Oracle:** `git-lfs/3.7.1 (GitHub; windows amd64; go 1.25.1; git b84b3384)`
- **Concurrency Invariant:** `-j 2` bounded compilation/execution.

---

## 2. Test Suite Inventory

| Test Identifier | File | Scenario / Verifications | Oracle Type | Status |
|---|---|---|---|---|
| `test_git_smart_http_push_clone_roundtrip` | `git_smart_http_test.rs` | Initial push -> clone -> commit -> second push -> second clone. Exact file tree & commit hashes match. | Official Git CLI | **PASS** |
| `test_git_smart_http_private_repository_authentication` | `git_smart_http_test.rs` | Anonymous clone fails (401). PAT-authenticated clone succeeds. | Official Git CLI | **PASS** |
| `test_git_smart_http_branch_protection_rejection` | `git_smart_http_test.rs` | Direct push to protected branch rejected on server with `require_pull_request`. | Official Git CLI | **PASS** |
| `test_git_smart_http_secret_scanning_rejection` | `git_smart_http_test.rs` | Push introducing plaintext AWS access key rejected on server. | Official Git CLI | **PASS** |
| `test_git_smart_http_url_without_dot_git_and_tags` | `git_smart_http_test.rs` | Bare URL without `.git` suffix handles pushes, clones, and tags. | Official Git CLI | **PASS** |
| `test_git_smart_http_fetch_pull_lifecycle` | `git_smart_http_test.rs` | Multi-step lifecycle: clone -> 2nd client push -> `git fetch` -> `git pull` -> tag fetch -> branch fetch -> repeat fetch. | Official Git CLI | **PASS** |
| `test_git_lfs_batch_api_conformance` | `git_lfs_http_test.rs` | 10 conformance cases: unsupported adapters (422), unsupported hash algos (422), upload batch, streamed PUT (5 MiB), verify POST, deduplication (omit actions), download batch, streamed GET, 404 missing, tampered token (403). | Tower / ServiceExt | **PASS** |
| `test_git_lfs_receive_pack_pre_receive_rejection` | `git_lfs_http_test.rs` | Pushing commit with LFS pointer without uploading binary asset rejected by pre-receive verification hook. | Official Git CLI | **PASS** |
| `test_official_git_lfs_workflow` | `git_lfs_http_test.rs` | End-to-end official `git` + `git-lfs` 3.7.1: `git init`, `git lfs track`, commit 5 MiB binary, `git push`, fresh clone with byte-for-byte binary reconstruction, modify binary, push, pull. | Official Git + Git LFS | **PASS** |

---

## 3. Performance & Resource Baselines

- **LFS 5 MiB Upload & Chunking:** 80–120 ms (includes SHA-256 validation, BLAKE3 FastCDC chunking, on-disk writes).
- **LFS 5 MiB Download & Reconstruction:** 15–25 ms (reassembled from 2 chunks in `LocalArtifactStore`).
- **Memory Footprint:** Peak RSS < 45 MiB during 5 MiB transfer.
- **Git Smart HTTP Fetch/Pull Turnaround:** 12–25 ms per fetch cycle.
