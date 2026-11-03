# Spine Forge: Multi-Tenant Isolation and Security Architecture

## 1. Overview

In multi-tenant SaaS environments, security boundaries must be enforced without compromise. Spine Forge handles sensitive corporate codebases, proprietary intellectual property, and access keys. This document outlines the defense-in-depth security principles implemented in the Forge collaboration backend.

---

## 2. Core Security Pillars

### 1. Metadata Concealment (`404 Not Found` vs `403 Forbidden`)
A classic vulnerability in multi-tenant systems is information disclosure via differential status codes. If an unauthenticated user queries `/api/v1/repos/secret-org/stealth-project`:
- Returning `403 Forbidden` leaks that a repository named `stealth-project` exists under `secret-org`.
- Spine Forge strictly returns `404 Not Found` whenever a user lacks authorization to observe a private resource.
- This applies consistently to:
  - Private repositories
  - Private organizations
  - Non-public pull requests, issues, and diffs
  - Protected branch rule configurations

### 2. Password & Credential Hardening
- Passwords are encrypted using Argon2id with recommended OWASP memory hardness (19 MiB memory, 2 iterations, 1 parallelism).
- Personal Access Tokens (PATs) and session tokens use cryptographically secure 256-bit random entropy (`OsRng`).
- Raw tokens are never stored in the database. Only SHA-256 hashes (`token_hash`) are persisted.
- Leaked tokens or revoked sessions are invalidated instantly in memory and PostgreSQL.

### 3. Cross-Tenant Data Fencing
All queries in PostgreSQL and the in-memory adapter enforce relational tenant boundaries:
- Team permissions are strictly checked against `repo_id` AND `team.org_id == repo.owner_org_id`.
- Users cannot be assigned permissions on repositories outside their organizational boundaries unless added as explicit external collaborators.
- Foreign key constraints with `ON DELETE CASCADE` ensure no orphaned permission grants linger after a team or organization is removed.

### 4. Code Review Self-Approval Defense
A user proposing a pull request cannot approve their own PR or forge approvals from other accounts. Branch protection rules verify that approvals originate from distinct, authorized accounts with write or maintain privileges.

### 5. Atomic Ref Synchronization & Lease Guarantees
Merge operations on repositories execute atomic three-way merges with two-parent commit objects. Updates to `.spine/refs/heads/<base>` are performed using atomic lock files (`.lock`) to prevent race conditions during concurrent merges or pushes.
