# SPINE SECURITY MODEL & THREAT ANALYSIS

## 1. Threat Matrix & Mitigations

| Threat Category | Potential Attack Vector | Spine Mitigation Architecture |
|---|---|---|
| **Path Traversal** | Malicious commit or branch containing `../` or absolute Windows/POSIX paths attempting to write outside working directory. | **Strict Path Validation:** Normalized relative paths. Rejection of components with `..`, leading slashes, Windows colons (`:`), or alternate data streams. |
| **Symlink Attacks** | Symlinks pointing to `/etc/shadow` or sensitive parent directories dereferenced during checkout or status. | **Sandboxed Symlinks:** Symlinks are recorded as metadata objects. Resolving symlinks outside repo root is prohibited. |
| **Type Confusion in Objects** | Crafting a blob whose raw byte content matches a tree or commit object to spoof repository state. | **Cryptographic Domain Separation:** Hashing prepends `SPINE\0<type>\0<len>\0`. A blob payload cannot produce the same hash as a tree or commit. |
| **Object Tampering** | Bit-rot, disk corruption, or malicious disk modification of loose objects. | **Cryptographic Verification on Read:** Object stores verify payload hash against `ObjectId` upon retrieval. `spine fsck` provides full tree validation. |
| **Windows File Lock Hijacking** | Attempting to overwrite open files resulting in Windows locking errors or race conditions. | **Tempfile Swaps with Atomic POSIX Semantics:** Writes occur to dedicated UUID files in `.spine/tmp/` followed by atomic replacement. |
| **Denial of Service / Unbounded Allocation** | Crafting malformed object headers with 64-bit integer lengths causing out-of-memory crashes. | **Bounded Allocation Checks:** Payload lengths are verified against system limits before memory allocation. Maximum object size enforced. |
| **Secret & Credential Exposure** | Passwords, private keys, or API tokens accidentally exposed in plain text logs or commits. | **Secure Defaults:** Passwords hashed with Argon2id. Pre-commit hooks for secret pattern detection in Milestone 4. |

---

## 2. Authentication & Authorization (Forge)
- **Token Format:** Cryptographically signed tokens (`sp_pat_...`) with constant-time comparison to prevent timing attacks.
- **Repository Access:** Strict role-based access control (Owner, Admin, Write, Read) evaluated at every API entrypoint.
- **Sandboxed Execution:** Future CI/CD runners run in ephemeral, unprivileged Linux/Windows containers with no access to host storage volumes.
