# SPINE FORGE SECURITY MODEL

## 1. Authentication & Session Management
- **Password Storage:** Argon2id with memory cost $65536 \text{ KiB}$, iteration count $3$, and parallelism factor $4$.
- **Session Tokens:** Cryptographically random 256-bit values stored hashed with SHA-256. Delivered via `HttpOnly`, `Secure`, `SameSite=Lax` cookies.
- **Personal Access Tokens (PATs):** Formatted with secure prefixes (`sp_pat_<40_alphanumeric_chars>`), hashed in the database, with granular scopes (`repo:read`, `repo:write`, `admin:org`).

## 2. Authorization (RBAC)
- **Levels:** `Owner`, `Admin`, `Maintainer`, `Collaborator`, `Reader`.
- Every internal and public API endpoint performs authorization checks prior to dispatching queries to storage nodes or databases.

## 3. Storage Node Isolation
- Storage nodes only accept authenticated, mTLS-secured connections from registered API gateways.
- Tenant path traversal is prohibited: storage paths are derived exclusively from numeric/UUID repository identifiers.
