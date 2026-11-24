# Spine Security Policy & Vulnerability Disclosure

Security is fundamental to Spine's architecture as a high-performance distributed version control system and enterprise collaboration platform. This document outlines our security policies, supported versions, vulnerability reporting mechanisms, and system threat model.

---

## 1. Supported Versions

Security fixes and critical patches are actively provided for the following releases:

| Component | Version / Branch | Supported | Notes |
|:---|:---|:---|:---|
| Spine VCS (`spine` CLI & crates) | `main` | :white_check_mark: | Primary development branch |
| Spine Forge (`backend` & `frontend`) | `main` | :white_check_mark: | Production candidate stream |
| Historical minor releases | `< 0.2.0` | :x: | End-of-life; please upgrade to `main` |

---

## 2. Reporting a Vulnerability

If you discover a security vulnerability in Spine, please report it **confidentially**. Do not disclose security vulnerabilities publicly or open public GitHub issues until a fix has been prepared and released.

### 2.1 Private Reporting Channels
- **GitHub Private Vulnerability Reporting:** Submit a report directly through GitHub's Security Advisories tab under the Spine repository.
- **Security Contact Email:** Send encrypted details to `security@spine-vcs.org`.

### 2.2 What to Include
When reporting a vulnerability, please provide:
1. A clear description of the vulnerability, including affected components (`git-alternative`, `github-alternative/backend`, or `github-alternative/frontend`).
2. Step-by-step instructions or a minimal reproducible proof-of-concept (PoC).
3. The potential impact (e.g., unauthorized data disclosure, path traversal, privilege escalation, or denial of service).
4. Any proposed remediations or patches.

### 2.3 Response & Disclosure Process
- **Initial Acknowledgment:** Within 48 hours of receipt.
- **Triaging & Reproduction:** Within 5 business days, we confirm findings and assign severity (CVSS score).
- **Remediation & Testing:** Patches are developed in private branches and validated against our regression test matrix.
- **Coordinated Disclosure:** We coordinate release announcements with reporters, typically within 90 days of the initial report, or sooner upon mutual agreement.

---

## 3. Threat Model & Security Boundaries

Spine implements defense-in-depth across the CLI engine, network services, database layer, and storage cluster.

### 3.1 Path Traversal & Filesystem Sandboxing
- **Relative Path Confinement:** All paths staged, checked out, or extracted by the CLI and backend workers are strictly validated. Paths containing `..`, absolute paths (e.g., `/etc` or `C:\Windows`), or Windows drive letters are rejected.
- **Windows Alternate Data Streams:** Windows NTFS alternate streams (`filename::$DATA`) and reserved device names (`CON`, `PRN`, `AUX`, `NUL`, `COM1-9`, `LPT1-9`) are sanitized or rejected.
- **Symlink Protection:** Symbolic links are tracked as raw metadata objects and are never dereferenced across repository boundaries during working-tree scans or archive creation.

### 3.2 Cryptographic Integrity & Domain Separation
- **Object Hashing:** Object IDs use BLAKE3 (256-bit digest) by default, providing high performance and cryptographic resistance against collision and length-extension attacks.
- **Type Domain Separation:** Object serialization hashes include explicit domain separation headers (`SPINE\0<type_byte>\0<payload_len_u64_le>\0`), ensuring that a raw blob cannot be synthetically masqueraded as a commit or tree object.
- **Tamper Evidence:** Any modification to an object payload or reference pointer immediately breaks the Merkle tree invariant and fails repository verification (`spine fsck`).

### 3.3 Authentication & Scoped Capability Tokens
- **Password Security:** User credentials are encrypted with Argon2id using salt and memory-hard computational parameters.
- **Personal Access Tokens (PAT):** PATs are stored exclusively as SHA-256 hashes in PostgreSQL; plaintext tokens are revealed only once at creation time.
- **Scoped Capability Tokens:** Short-lived tokens for Git LFS and artifact upload/download are cryptographically bound to:
  - Permitted operation (`upload` vs `download`).
  - Specific repository identifier (`repository_id`).
  - Target object digest (`oid`).
  - Expiration timestamp.
  Cross-repository reuse, scope confusion, or expired tokens are rejected with HTTP 403 Forbidden.

### 3.4 Multi-Tenant Storage & CAS Isolation
- **Global CAS Deduplication with Strict Logical Isolation:** Content-Addressable Storage (CAS) chunks are deduplicated globally across repositories for storage efficiency. However, object discovery, batch queries, download endpoints, and verify operations strictly check the repository mapping table (`lfs_objects`).
- **No Existence Leaks:** A tenant cannot discover or verify the presence of an object belonging to another repository, even if both repositories utilize identical chunks.

### 3.5 In-Flight Upload Protection & GC Safety
- **Durable Upload Leases:** To prevent Garbage Collection (GC) workers from pruning chunks during long-running concurrent uploads, active uploads register durable, heartbeat-renewed filesystem leases via `InFlightUploadRegistry`.
- **Mtime Grace Period:** GC applies an additional configurable grace period (default 24h) before tombstoning or deleting unreferenced CAS chunks.

### 3.6 Backup & Disaster Recovery (SPNEBAK v2)
- **Authenticated Encryption (AEAD):** Disaster recovery backup archives are encrypted with authenticated encryption (XChaCha20-Poly1305 or AES-256-GCM), guaranteeing confidentiality and tamper-proofing against unauthorized archive tampering.
- **Consistency Maintenance Barrier:** Backups and point-in-time restores enforce a cluster maintenance barrier to guarantee that CAS chunks and database metadata remain in a durable, consistent state.

---

## 4. Production Hardening Requirements

Deployments in production environments must enforce the following prerequisites:
1. **TLS / HTTPS:** All external HTTP and WebSocket traffic must terminate over TLS 1.3 or 1.2 with secure cipher suites.
2. **Database Isolation:** PostgreSQL must run in an isolated VPC with restricted network access and encrypted connections (`sslmode=verify-full`).
3. **Storage Immutability:** Enterprise deployments should configure WORM (Write Once, Read Many) or Object Lock (e.g., AWS S3 Object Lock) on backup destinations.
4. **Secret Scanning:** All CI/CD pipelines enforce automated Gitleaks secret scanning rules.
