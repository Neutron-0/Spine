# BATCH 17 — PHASE ZERO GAP REGISTER
## Native Git LFS HTTP API, Smart HTTP Hardening, and Complete Large-File Remote Compatibility

**Date:** 2026-11-04  
**Project:** Spine VCS & Spine Forge  
**Previous Baseline Commit:** `c9629f4` (`feat(batch-16): native git smart http, remote clone/fetch/push, and forge transport integration`)  
**Status:** AUDITED & GAPS RECORDED FOR IMPLEMENTATION

---

### 1. Smart HTTP Fetch and Pull Lifecycle Evidence (Section 1.1)

- **Audit Findings:**
  - In Batch 16, integration tests in `git_smart_http_test.rs` proved remote push and clone round-trips (`test_git_smart_http_push_clone_roundtrip`), private authentication, branch protection, and secret scanning.
  - However, the test did not explicitly isolate the full multi-step `git fetch` and `git pull` lifecycle on an existing clone when the remote receives external updates from another client.
  - Specifically, the following sequence was not verified under an automated oracle:
    1. Remote updated by a second client on an existing branch.
    2. Original client runs `git fetch` -> inspects `FETCH_HEAD` and remote-tracking ref.
    3. Original client runs `git pull` -> fast-forward merge and tree/ancestry verification.
    4. Remote adds annotated and lightweight tags -> original client runs `git fetch --tags` -> tag resolution verification.
    5. Remote adds an additional feature branch -> original client runs `git fetch origin feature` -> branch ref creation.
    6. Repeated `git fetch` with zero new changes -> no-op verification without ref corruption or duplicate object synthesis.
- **Required Remediation:**
  - Implement `test_git_smart_http_fetch_pull_lifecycle` in the integration test suite executing all 6 steps using official `git.exe` (`git 2.53.0.windows.1`).
- **Status:** **OPEN — IMPLEMENTING IN BATCH 17**

---

### 2. Git Object Mapping Durability & Crash Consistency (Section 1.2)

- **Audit Findings:**
  - Batch 16 introduced `.spine/git_map.json` and cached raw Git objects in `.spine/git_objects/<oid>`.
  - While these files are repository-scoped and survive normal API restarts, `save_git_map` used direct `fs::write(&map_file, json)`. If the process terminates abruptly during write, the JSON file could be torn or corrupted.
  - Furthermore, if a Spine commit is created natively (via Spine CLI, web UI editor, or PR merge) without passing through `git-receive-pack`, `git_map.json` does not initially contain its Git OID.
- **Required Remediation:**
  - Harden `save_git_map` to write atomically via `.tmp` staging file and atomic file rename (`fs::rename`).
  - Implement `reconcile_git_map(spine_dir)`: a recovery and reconciliation function that scans all native Spine refs and commits, synthesizes any missing Git OIDs using `GitExporter`, and atomically updates `git_map.json`.
  - Ensure Git mapping updates and ref publication maintain crash consistency.
- **Status:** **OPEN — IMPLEMENTING IN BATCH 17**

---

### 3. Smart HTTP Capability Verification & Protocol Negotiation (Section 1.3)

- **Audit Findings:**
  - Inspection of `advertise_refs_upload_pack` shows capabilities: `symref=HEAD:refs/heads/{default} multi_ack_detailed no-done side-band-64k ofs-delta agent=spine/0.1.0`.
  - Inspection of `advertise_refs_receive_pack` shows capabilities: `report-status delete-refs ofs-delta agent=spine/0.1.0`.
  - All advertised capabilities correspond to verified protocol v0/v1 behavior in `pkt_line.rs` and `git_transport.rs`.
  - Protocol v2 is *not* implemented and is *not* advertised.
- **Required Remediation:**
  - Keep protocol v2 explicitly unadvertised. Document the protocol limitation clearly in `docs/GIT_SMART_HTTP_PROTOCOL_SUPPORT.md` and `docs/ROADMAP.md`.
- **Status:** **VERIFIED — INVARIANTS MAINTAINED**

---

### 4. Canonical Public HTTPS Origin & Plaintext Credential Rejection (Section 1.4)

- **Audit Findings:**
  - Batch 16 clone URLs in `App.tsx` were generated from `window.location.origin` (or request `Host` header). In external environments, HTTP Basic authentication over plaintext HTTP exposes user credentials.
- **Required Remediation:**
  - Introduce configurable canonical public origin `public_origin` (via `FORGE_PUBLIC_ORIGIN`, `APP_URL`, or CLI flag, defaulting to `https://forge.example.com` in production).
  - Enforce transport encryption policy: If `public_origin` is HTTPS or `SPINE_REQUIRE_HTTPS=1`, reject authenticated remote Git and Git LFS requests from non-loopback addresses arriving over plaintext HTTP with `403 Forbidden`.
  - Permit plaintext HTTP exclusively for loopback addresses (`127.0.0.1`, `localhost`, `::1`) during local development and automated integration testing.
  - Prohibit placing PAT tokens in clone URLs or query strings.
  - Zero logging of `Authorization` headers.
- **Status:** **OPEN — IMPLEMENTING IN BATCH 17**

---

### 5. Git & LFS Authentication Scopes (Section 1.5)

- **Audit Findings:**
  - `authenticate_git_user` verifies PATs, active sessions, and Argon2id passwords.
  - PATs are the recommended mechanism for Git remote transports.
- **Required Remediation:**
  - Enforce token expiration and revocation checks.
  - Enforce read permissions on upload-pack and LFS download operations; enforce write permissions on receive-pack and LFS upload operations.
  - Protect private repository existence (return 401/404 for unauthenticated callers).
- **Status:** **OPEN — IMPLEMENTING IN BATCH 17**

---

### 6. Standard Git LFS Basic Transfer Protocol (Sections 2–5)

- **Audit Findings:**
  - Batch 11 implemented a custom LFS endpoint at `/api/v1/repos/:owner/:repo/lfs/objects/batch` with custom chunk sessions (`chunks/:index/:hash` and `finalize`).
  - Standard `git-lfs` clients fail against this prototype because:
    1. Standard endpoint path is `/{owner}/{repo}.git/info/lfs/objects/batch` (or without `.git`).
    2. Git LFS expects the standard `"basic"` transfer adapter:
       - Upload action: single HTTP `PUT` to `upload.href` with the raw object bytes, followed by `POST` to `verify.href`.
       - Download action: single HTTP `GET` from `download.href` returning raw object bytes.
    3. Git LFS expects `Content-Type: application/vnd.git-lfs+json`.
    4. Objects already present in storage must be returned *without* actions in upload responses so `git-lfs` skips re-uploading them.
- **Required Remediation:**
  - Mount standard Git LFS routes at `/:owner/:repo_git/info/lfs/objects/batch`, `/:owner/:repo_git/info/lfs/objects/:oid`, and `/:owner/:repo_git/info/lfs/verify`.
  - Implement full Git LFS Batch API with `basic` transfer adapter, returning scoped short-lived HMAC transfer capabilities.
  - Implement streamed `PUT` upload into `spine-artifact`, computing SHA-256 and BLAKE3, creating manifests, and updating repository quotas.
  - Implement `POST` verification confirming persisted manifest and size.
  - Implement streamed `GET` download returning raw binary bytes.
  - Integrate pre-receive hook in `git_receive_pack`: inspect newly introduced blobs for `GitLfsPointer` and reject commits whose backing LFS objects have not been uploaded!
- **Status:** **OPEN — IMPLEMENTING IN BATCH 17**

---

### 7. Carried Forward Environment Limitations (Section 1.7)

- **PostgreSQL Live-Server Verification:** Remains **BLOCKED** on host Windows environment without local Docker daemon or running PostgreSQL service. SQLite/in-memory adapter tested with full coverage; PostgreSQL adapter code-audited.
- **Windows Runner Sandbox Isolation:** Remains **FAIL-CLOSED** without hardware virtualization or container isolation.
- **Storage Consensus:** Multi-node consensus protocols remain scheduled for future clustering phases.
