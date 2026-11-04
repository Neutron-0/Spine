# Spine Forge — Git Smart HTTP Security Architecture

## 1. Attack Vectors and Defenses

| Threat Vector | Mitigation Strategy | Enforcement Layer |
|---|---|---|
| **Plaintext Credential Exposure** | Reject Basic Auth over unencrypted HTTP outside loopback addresses (`127.0.0.1`, `localhost`, `[::1]`). | `routes_lfs::check_transport_security` |
| **Credential Token Leakage** | Zero Personal Access Tokens (PATs) in URLs or query parameters. Zero `Authorization` logging. | Middleware / Routing / Logging |
| **Secret Introduction in Commits** | Pre-receive blob scanner inspects all newly reachable Git blobs for PATs, private keys, and cloud tokens before ref updates. | `git_receive_pack` pre-receive scanner |
| **Missing LFS Binary Attacks** | Pre-receive verification verifies that all Git LFS pointer blobs have their corresponding binary objects stored in `spine-artifact`. | `git_receive_pack` LFS verifier |
| **Non-Fast-Forward Overwrites** | Protected branch rules enforce `is_ancestor` fast-forward checks and pull request requirements. | Authoritative CAS write pipeline |
| **Resource Exhaustion (Denial of Service)** | Maximum packfile body limits, object count caps (500 objects/batch), chunk stream bounding. | Axum `DefaultBodyLimit` (1 GiB) |

---

## 2. Pre-Receive Hook Verification Pipeline

When a Git client initiates a `git-receive-pack` push:
1. The thin pack is unpacked into native objects, delta-resolved against existing repository trees/commits, and converted to Spine internal representations.
2. Newly created blobs are passed through the security scanner:
   - Evaluates high-confidence regex patterns for API keys, AWS credentials, RSA keys, and Spine PATs.
   - If a secret is found, push is rejected: `push rejected: secret scanning detected sensitive credentials (<type>)`.
3. Newly created blobs are evaluated for Git LFS pointers:
   - Validates prefix: `version https://git-lfs.github.com/spec/v1\n`.
   - Extracts `oid sha256:` and `size`.
   - Queries `state.db` / `spine-artifact` store to confirm the backing binary artifact is stored.
   - If missing, push is rejected: `push rejected: missing referenced LFS object '<oid>' (<size> bytes); upload via git-lfs before pushing`.
4. Branch protection rules are evaluated:
   - Checks `require_pull_request`.
   - Checks `is_ancestor` for fast-forward verification.
5. Atomic CAS Reference Updates:
   - Compares expected `old_oid` with the current branch reference.
   - Updates reference atomically via atomic file write.
