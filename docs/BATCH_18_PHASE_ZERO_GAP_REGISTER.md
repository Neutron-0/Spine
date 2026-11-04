# BATCH 18 — PHASE ZERO AUDIT AND GAP REGISTER
**Timestamp:** 2026-11-04  
**Workspace:** `d:\Vault\Spine`  
**Milestone Target:** Batch 18 — Native Git-over-SSH, SSH Key Management, Git LFS Authentication and File Locking  
**Baseline Commit:** `cbb97f9`

---

## 1. Environment & Tooling Verification

| Tool / Subsystem | Host Version / Status | Verification Result |
| :--- | :--- | :--- |
| **OpenSSH Client** | `OpenSSH_for_Windows_9.5p2, LibreSSL 3.8.2` | **VERIFIED** — Ready for real client test oracles (`ssh`, `ssh-keygen`, `ssh-agent`) |
| **Git CLI** | `git version 2.53.0.windows.1` | **VERIFIED** — Standard Git CLI client |
| **Git LFS** | `git-lfs/3.7.1 (GitHub; windows amd64; go 1.25.1; git b84b3384)` | **VERIFIED** — Standard Git LFS client |
| **Docker Engine** | Docker pipe unavailable (daemon not running) | Recorded (In-memory DB test harness active) |
| **PostgreSQL Service** | No local Windows service running | Recorded (In-memory DB test harness active; `PostgresDatabase` compiles) |
| **Windows Runner Isolation** | Windows Job Objects active | Maintained from Batch 12-14 |

---

## 2. Regression Status (Batch 17 Baseline)

Executed tests on backend workspace (`cargo test --manifest-path github-alternative/backend/Cargo.toml -p spine-api -j 2`):
- `tests/git_lfs_http_test.rs`: 3 passed, 0 failed.
  * `test_git_lfs_receive_pack_pre_receive_rejection` (PASSED)
  * `test_git_lfs_batch_api_conformance` (PASSED)
  * `test_official_git_lfs_workflow` (PASSED)
- `tests/git_smart_http_test.rs`: 6 passed, 0 failed.
  * `test_git_smart_http_branch_protection_rejection` (PASSED)
  * `test_git_smart_http_private_repository_authentication` (PASSED)
  * `test_git_smart_http_secret_scanning_rejection` (PASSED)
  * `test_git_smart_http_url_without_dot_git_and_tags` (PASSED)
  * `test_git_smart_http_push_clone_roundtrip` (PASSED)
  * `test_git_smart_http_fetch_pull_lifecycle` (PASSED)

All 9 end-to-end transport tests pass cleanly without regression.

---

## 3. Batch 18 Gap Register and Implementation Strategy

### Gap 1: Pure-Rust Native Git-over-SSH Server
- **Current State:** Spine Forge only accepts Git clients over HTTPS (Git Smart HTTP).
- **Target State:** Integrated pure-Rust SSH daemon using `russh`:
  - Ephemeral port support in tests and configurable server port (e.g. 2222) in production.
  - Zero external process spawning: no system SSH daemon, no `sshd`, no shell (`cmd.exe`, `powershell`, `/bin/sh`).
  - Persistent Ed25519 host key stored securely under `storage_root/ssh/host_key_ed25519`.
  - Automatic host key generation on initial launch if absent.
  - Strict exec restriction: only `git-upload-pack`, `git-receive-pack`, and `git-lfs-authenticate` commands allowed.
  - Shell access, PTY allocation, port forwarding, and X11 forwarding rejected immediately.

### Gap 2: SSH Key Management & Authorization
- **Current State:** User authentication relies solely on passwords, session cookies, and Personal Access Tokens (PAT).
- **Target State:**
  - `UserSshKey` entity: allows users to register public SSH keys (Ed25519, RSA, ECDSA), compute fingerprints (SHA256), track `last_used_at_nanos`, and revoke.
  - `DeployKey` entity: repository-scoped public keys with explicit `write_enabled: bool` permission flag.
  - SSH public key authentication (`russh::server::Server::auth_publickey`) validates incoming public keys against both user keys and deploy keys.
  - Scoped identity attached to session channel: read or write permissions mapped onto existing repository authorization, branch protection, and secret scanning pipeline.

### Gap 3: Git LFS SSH Authentication (`git-lfs-authenticate`)
- **Current State:** Git LFS clients communicate via HTTPS with Basic Auth or Bearer tokens. When a remote is configured with an SSH URL (`git@host:owner/repo.git`), Git LFS automatically calls `ssh git@host "git-lfs-authenticate <repo-path> <operation>"`.
- **Target State:**
  - Command dispatcher detects `git-lfs-authenticate <repo> <upload|download>`.
  - Generates standard Git LFS response payload:
    ```json
    {
      "href": "https://<public_origin>/<owner>/<repo>.git/info/lfs",
      "header": {
        "Authorization": "RemoteAuth <scoped_hmac_token>"
      },
      "expires_in": 3600
    }
    ```
  - Git LFS HTTP endpoint (`routes_lfs.rs`) supports `Authorization: RemoteAuth <scoped_hmac_token>` in addition to `Bearer` and `Basic`.

### Gap 4: Git LFS File Locking API
- **Current State:** Git LFS Batch API and Basic Transfer are implemented, but File Locking endpoints are missing.
- **Target State:**
  - Implement full LFS File Locking specification under `/:owner/:repo_git/info/lfs/locks`:
    * `POST /info/lfs/locks`: Create lock on normalized relative path, prevent duplicate locks.
    * `GET /info/lfs/locks`: Query locks with filtering by `path`, `id`, `cursor`, `limit`.
    * `POST /info/lfs/locks/verify`: Categorize locks into `ours` and `theirs` for pre-push verification.
    * `POST /info/lfs/locks/:id/unlock`: Unlock by owner or admin force unlock (`force: true`).
  - Pre-receive lock enforcement in `git-receive-pack`:
    * Parse incoming commit tree changes in received packfile.
    * Verify that none of the modified paths are currently locked by a different user.
    * Reject push with clear error message if conflicting lock exists.

### Gap 5: Frontend Interface
- **Current State:** Frontend UI supports HTTPS clone URLs and basic repository settings.
- **Target State:**
  - Account Settings: SSH Keys panel (list, register with title & public key, delete/revoke).
  - Repository Settings: Deploy Keys panel (list, add with title, key, write toggle, delete/revoke).
  - Clone Modal: HTTPS / SSH clone URL toggle with instant copy-to-clipboard.
  - Repository Navigation: "LFS Locks" tab/view displaying locked files, lock owners, creation dates, and unlock/force-unlock actions.
