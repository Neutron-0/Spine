# SPINE SECURITY POLICY

## 1. Security Architecture Principles

Spine is designed with defense-in-depth across the version control engine and collaboration platform:

### 1.1 Path Traversal Protection
- All working tree paths staged or accessed via CLI commands are strictly validated against directory escape patterns:
  - Absolute paths (e.g. `C:\Windows\...`, `/etc/...`) are rejected in relative path contexts.
  - Path components containing `..` or leading slashes are stripped or rejected.
  - Windows drive letters and alternate data streams (`filename::$DATA`) are neutralized.

### 1.2 Symlink Sandboxing
- Symlinks are treated as metadata and never dereferenced during repository traversal unless explicitly resolved inside the repository root.
- Cross-boundary symlinks targeting parent or system paths are refused tracking.

### 1.3 Memory Safety and Input Validation
- Written purely in safe Rust to eliminate memory corruption bugs (buffer overflows, use-after-free, double frees).
- Strict length limits on untrusted inputs: commit messages, author strings, configuration keys, and file paths have upper-bound validation to prevent memory exhaustion attacks.
- Integer arithmetic is checked or bounded against integer overflow.

### 1.4 Cryptographic Hashing
- Object IDs utilize BLAKE3 (256-bit digest) by default, providing high cryptographic resistance against length-extension, preimage, and collision attacks.
- Domain separation prefixes (`SPINE\0<type>\0`) guarantee that blobs cannot masquerade as trees or commits.

### 1.5 Forge Security Posture
- Secrets and tokens hashed using modern cryptographic algorithms (Argon2id for passwords, SHA-256 for API tokens).
- Repository access controls enforced at both API gateway and storage node RPC boundaries.
- Ephemeral sandboxed runner environments for CI/CD prevents lateral movement to storage nodes or API services.
