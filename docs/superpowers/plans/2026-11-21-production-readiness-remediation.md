# Production Readiness Remediation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

## Overview
Remediate release-blocking production readiness gaps across backup/restore maintenance barrier coordination, administrator credential flows, authenticated Git SSH clone/push verification, immutable image release packaging, and off-site disaster recovery verification.

---

### Task 1: Implement & Register Maintenance Barrier HTTP Endpoints (`/api/v1/admin/maintenance/acquire` & `/api/v1/admin/maintenance/release`) and Diagnostics Fields
- [ ] In `github-alternative/backend/apps/spine-api/src/backup/consistency.rs`, add getter `pub fn in_flight_writes(&self) -> usize`.
- [ ] In `github-alternative/backend/apps/spine-api/src/api/middleware.rs`:
  - Add `pub active_maintenance: Arc<tokio::sync::Mutex<Option<crate::backup::MaintenanceGuard>>>` to `AppState`.
- [ ] Create `github-alternative/backend/apps/spine-api/src/api/routes_admin.rs`:
  - Implement `acquire_maintenance` (`POST /api/v1/admin/maintenance/acquire`):
    - Authenticate admin token (`SPINE_ADMIN_TOKEN`, `SPINE_ADMIN_TOKEN_FILE`, or admin session/PAT).
    - Acquire maintenance lease from `consistency_barrier`.
    - Propagate fencing generation to coordinator and all storage nodes (`enter_maintenance(gen)`).
    - Store active `MaintenanceGuard` in `state.active_maintenance`.
    - Return JSON `{"status": "acquired", "fencing_generation": gen, "owner_token": guard.owner_token(), "lease_expires_at_nanos": ...}`.
  - Implement `release_maintenance` (`POST /api/v1/admin/maintenance/release`):
    - Authenticate admin token.
    - Release `MaintenanceGuard` from `state.active_maintenance`.
    - Propagate release to coordinator and storage nodes (`exit_maintenance(gen)`).
    - Return JSON `{"status": "released"}`.
- [ ] In `github-alternative/backend/apps/spine-api/src/api/routes_health.rs`:
  - Add `in_maintenance: bool`, `active_writers: usize`, and `current_fencing_generation: u64` to `DiagnosticsResponse` and populate them from `state.consistency_barrier`.
- [ ] Register routes in `github-alternative/backend/apps/spine-api/src/api/mod.rs`.
- [ ] Update `scripts/backup_oob.sh` to support `SPINE_ADMIN_TOKEN_FILE`.
- [ ] Write integration test verifying acquire/release lifecycle, writer draining, and 503 rejection during maintenance.

---

### Task 2: Administrator Bootstrap and Secure Token Acquisition Flow
- [ ] In `github-alternative/backend/apps/spine-api/src/main.rs`:
  - Update `bootstrap-admin` command to generate and print a dedicated personal access token (PAT) for administrative scripting (`sp_pat_...`), saving it to the database with no expiry.
  - Support `--token-out <FILE>` to securely write the generated token to a file without printing it in terminal logs.
- [ ] Test the bootstrap flow against the live database and verify token authentication with `/api/v1/admin/maintenance/acquire`.

---

### Task 3: Authenticated Git SSH Clone and Push Verification
- [ ] Using the provisioned administrator or API, register a test user `test-developer` and generate an Ed25519 SSH keypair.
- [ ] Add the public key via `POST /api/v1/users/me/ssh_keys`.
- [ ] Create a repository `test-developer/rehearsal-repo`.
- [ ] Perform authenticated Git clone and push over port 2222:
  - `git clone ssh://git@127.0.0.1:2222/test-developer/rehearsal-repo.git`
  - Commit a test payload and `git push origin main`.
- [ ] Verify pack negotiation, ref update, and PostgreSQL audit logging.

---

### Task 4: Base Image Digest Pinning and Immutable Release Packaging (P2)
- [ ] Pin base images in `Dockerfile.coordinator`, `Dockerfile.storage-node`, `Dockerfile.api`, `Dockerfile` (frontend), and `docker-compose.prod.yml` with explicit `@sha256:...` digests.
- [ ] Replace `:latest` image tags in `docker-compose.prod.yml` with release candidate version tag `v0.1.0-rc3`.
- [ ] Record image tags and digests in `RELEASE_CANDIDATE_MANIFEST.md` and `docs/RELEASE_PROVENANCE_AND_SBOM.md`.

---

### Task 5: Required Off-Site S3 Replication & Disaster Recovery Execution / Status
- [ ] Verify AWS CLI installation on host.
- [ ] Run out-of-band backup test with S3 replication gate.
- [ ] If AWS credentials/bucket are configured, perform encrypted backup, verify Compliance Object Lock, remote SHA-256 verification, and disposable volume restore.
- [ ] If live S3 compliance bucket is unavailable, record the exact command and missing dependency, explicitly keeping disaster recovery marked unverified at runtime.
