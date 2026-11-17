# Production Readiness Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remediate the critical P0/P1 security vulnerabilities, runtime data corruption bugs, core VCS merge defects, background daemons, and infrastructure blockers identified in the Spine production readiness audit.

**Architecture:** Target the critical blockers in four focused phases: (1) Cryptographic secrets, CORS, cookie security, and SSRF hardening; (2) Health probe side-effect elimination and background worker daemon spawning with HTTPS webhook delivery; (3) Core VCS 3-way hunk merge correction and linear-space diff allocation; (4) Docker healthcheck package tooling and database connection pool configurability.

**Tech Stack:** Rust (Axum, Tokio, SQLx, russh, blake3, zstd, ring/hmac), Docker, Nginx, TypeScript/React.

**Spec:** `SPINE_PRODUCTION_READINESS_AUDIT.md` (located in brain directory `C:/Users/Lakshya/.gemini/antigravity/brain/346134a0-9e77-42a2-bf1d-03a6cf0224cf/SPINE_PRODUCTION_READINESS_AUDIT.md`).

## Global Constraints

- Must not break any existing passing tests across `git-alternative` and `github-alternative/backend`.
- Zero compiler or clippy warnings (`cargo check`, `cargo clippy -- -D warnings`).
- Maintain `core-forge-v1` deployment profile security contract (fail-closed in production if required secrets are missing or insecure).

## Review Focus

1. `LFS_SECRET_SALT`: Must be dynamically provided per-deployment, rejecting fallback in `core-forge-v1`.
2. `/health/diagnostics`: Must be strictly read-only; scraping the endpoint must never modify row statuses in `webhook_outbox`.
3. Background workers: `WebhookOutboxWorker` and `WorkflowScheduler` must run continuously in `main.rs` and stop cleanly on SIGINT/Ctrl-C.
4. `merge_3way_text`: Must correctly merge non-overlapping edits in different parts of a file without line pointer desynchronization.
5. `compute_line_diff`: Must diff 10,000+ line files without allocating multi-gigabyte matrices.

---

### Task 1: Security Hardening (Secrets, CORS, Cookie Flags, SSRF)

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/api/routes_lfs.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/security/service.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/api/mod.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/api/routes_auth.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/api/middleware.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/webhook/ssrf.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/security_hardening_audit_test.rs`

**Interfaces:**
- Consumes: `AppState`, `DeploymentProfile`, `SsrfValidator`
- Produces: Dynamic LFS salt resolution, restricted CORS layer, secure cookie attributes, cluster-shielded SSRF validator.

- [ ] **Step 1: Write the failing test for security hardening**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement dynamic LFS salt, CORS restriction, secure cookies, and SSRF cluster port blocking**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing tests pass and commit**

---

### Task 2: Fix Health Diagnostics Outbox Mutation & Abandonment

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/api/routes_health.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/traits.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/in_memory.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/postgres.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/health_diagnostics_readonly_test.rs`

**Interfaces:**
- Consumes: `Database::count_pending_webhook_outbox()`
- Produces: Read-only `health_diagnostics` without claiming outbox items.

- [ ] **Step 1: Write the failing test proving `/health/diagnostics` mutates outbox items**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement `count_pending_webhook_outbox` and update `health_diagnostics`**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing tests pass and commit**

---

### Task 3: Background Worker Daemons & Webhook HTTPS Delivery

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/webhook/worker.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/workflow/scheduler.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/main.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/webhook_worker_daemon_test.rs`

**Interfaces:**
- Consumes: `WebhookOutboxWorker`, `WorkflowScheduler`
- Produces: Continuous background processing loops with graceful shutdown handles in `main.rs`.

- [ ] **Step 1: Write unit tests for worker loops and HTTPS webhook handling**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement run loop on `WebhookOutboxWorker`, scheduler tick loop, and spawn in `main.rs`**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing tests pass and commit**

---

### Task 4: Fix Broken 3-Way Line Merge in `spine-merge`

**Files:**
- Modify: `git-alternative/crates/spine-merge/src/lib.rs`
- Test: `git-alternative/crates/spine-merge/tests/three_way_merge_alignment_test.rs`

**Interfaces:**
- Consumes: `compute_line_diff`
- Produces: Correct `merge_3way_text` aligning edit hunks without index desynchronization.

- [ ] **Step 1: Write tests reproducing line desynchronization and false conflicts on offset insertions**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement hunk-aligned 3-way diff merge algorithm in `merge_3way_text`**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing merge tests pass and commit**

---

### Task 5: Linear-Space Diff Allocation & Worktree Path Sanitization

**Files:**
- Modify: `git-alternative/crates/spine-diff/src/lib.rs`
- Modify: `git-alternative/crates/spine-format/src/lib.rs`
- Modify: `git-alternative/crates/spine-worktree/src/lib.rs`
- Test: `git-alternative/crates/spine-diff/tests/linear_space_diff_test.rs`

**Interfaces:**
- Consumes: `old_text`, `new_text`
- Produces: Linear-space Myers line differencer, bounded deserialization capacity, sanitized switch paths.

- [ ] **Step 1: Write test for large file diff memory consumption and tree entry path traversal**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Implement linear-space diff in `compute_line_diff`, capacity bound in `deserialize_tree`, and path check in `execute_switch`**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing diff/format/worktree tests pass and commit**

---

### Task 6: Infrastructure & Connection Pooling Fixes

**Files:**
- Modify: `git-alternative/Dockerfile.coordinator`
- Modify: `git-alternative/Dockerfile.storage-node`
- Modify: `github-alternative/backend/apps/spine-api/src/db/postgres.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/main.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/db_pool_configuration_test.rs`

**Interfaces:**
- Consumes: `DATABASE_MAX_CONNECTIONS`, `DATABASE_MIN_CONNECTIONS`, `DATABASE_ACQUIRE_TIMEOUT_SECS`
- Produces: Working Docker healthcheck tooling, configurable SQLx connection pooling.

- [ ] **Step 1: Write test verifying connection pool settings are honored from environment**
- [ ] **Step 2: Run test to verify it fails**
- [ ] **Step 3: Update Dockerfiles to include `curl` for healthchecks; implement `PgPoolOptions` configuration**
- [ ] **Step 4: Run test to verify it passes**
- [ ] **Step 5: Verify all existing tests pass and commit**
