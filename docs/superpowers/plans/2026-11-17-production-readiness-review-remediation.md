# Production Readiness Review Remediation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve all 5 release blockers and additional operational issues identified in the production readiness review, verify with TDD tests, commit the clean working tree, and package release candidate v0.1.0-rc3 with accurate checksums.

**Architecture:** 
1. Strict production secret enforcement in `main.rs` and unpredictable ephemeral fallback in `routes_lfs.rs` to eliminate `RemoteAuth` token forgery.
2. Proper non-overlapping range grouping in `spine-merge` to allow clean merging of independent adjacent-line edits.
3. Mozilla root certificate trust via `webpki-roots` for production HTTPS webhooks and full-delivery timeout wrapping.
4. Lease expiration and crash recovery in webhook outbox database queries.
5. FIFO active workflow run fetching to prevent run starvation and dual `SPINE_DB_*` / `DATABASE_*` configuration support.
6. Clean git commit and release binary rebuild with updated manifest.

**Tech Stack:** Rust 1.88, Axum, Tokio, SQLx, Rustls, WebPKI-Roots, Myers Diff.

**Spec:** Review verdict feedback from user.

---

## Global Constraints

- Never use fixed or source-visible static salts for cryptographic tokens or authentication.
- All webhook deliveries must have bounded execution time (connect, handshake, send, receive).
- Every fix must have an automated test demonstrating RED before the fix and GREEN after.
- Database queries must not starve older tasks or drop stranded items after interruption.
- Clean compilation without warnings or errors across all workspace crates.

---

## Review Focus

1. `RemoteAuth` token signed with default static salt must be rejected.
2. Two branches modifying adjacent lines (line 1 and line 2) must cleanly merge without conflict markers.
3. HTTPS webhooks sent without custom root configuration must accept valid public TLS certificates.
4. An outbox item left in `PROCESSING` status after a crash must be automatically reclaimed after lease expiration.
5. An older active workflow run must be scheduled even when more than 100 newer runs exist in the repository.

---

### Task 1: Authentication Security & Production Secret Enforcement

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/api/routes_lfs.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/api/routes_git_http.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/main.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/auth_secret_enforcement_test.rs`

**Interfaces:**
- Consumes: `SPINE_AUTH_SECRET_KEY`, `SPINE_LFS_SECRET`, `DeploymentProfile::CoreForge`
- Produces: `validate_production_secrets()`, secure dynamic salt with random ephemeral fallback.

- [x] **Step 1: Write failing test verifying default salt cannot forge tokens and production rejects missing secret**
- [x] **Step 2: Run test to verify it fails**
- [x] **Step 3: Implement `validate_production_secrets` in `main.rs` and replace static salt with `OnceLock` random ephemeral salt in `routes_lfs.rs`**
- [x] **Step 4: Run test to verify it passes**
- [x] **Step 5: Verify all auth tests pass**

---

### Task 2: Fix Adjacent-Line False Merge Conflicts in `spine-merge`

**Files:**
- Modify: `git-alternative/crates/spine-merge/src/lib.rs`
- Test: `git-alternative/crates/spine-merge/tests/three_way_merge_alignment_test.rs`

**Interfaces:**
- Consumes: Myers diff hunks `DiffHunk { base_start, base_end, replacement }`
- Produces: Correct range intersection test preventing adjacent non-overlapping edits from grouping into false conflicts.

- [x] **Step 1: Write test in `three_way_merge_alignment_test.rs` for adjacent line changes**
- [x] **Step 2: Run test to verify it fails**
- [x] **Step 3: Update hunk grouping logic in `merge_3way_text` so adjacent ranges do not overlap**
- [x] **Step 4: Run test to verify it passes**
- [x] **Step 5: Verify all `spine-merge` tests pass**

---

### Task 3: Production HTTPS Webhook Certificate Trust & Comprehensive Timeouts

**Files:**
- Modify: `git-alternative/crates/spine-net/src/tls.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/webhook/worker.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/webhook_tls_production_trust_test.rs`

**Interfaces:**
- Consumes: `webpki_roots::TLS_SERVER_ROOTS`, `timeout_duration`
- Produces: Default trusted Mozilla roots for client TLS and full-delivery timeout.

- [x] **Step 1: Write unit test verifying default TLS config loads WebPKI roots and delivery times out completely on hanging handshake**
- [x] **Step 2: Run test to verify it fails**
- [x] **Step 3: Update `build_client_tls_config` in `tls.rs` to load `TLS_SERVER_ROOTS` when roots are None, and wrap entire delivery in timeout in `worker.rs`**
- [x] **Step 4: Run test to verify it passes**
- [x] **Step 5: Verify all webhook tests pass**

---

### Task 4: Webhook Outbox Lease Expiry & Crash Recovery

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/db/traits.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/in_memory.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/postgres.rs`
- Test: `github-alternative/backend/apps/spine-api/tests/webhook_crash_recovery_test.rs`

**Interfaces:**
- Consumes: `claim_pending_webhook_outbox(limit, now_nanos)`
- Produces: 60-second lease timestamp on claimed items and automatic reclamation of expired `PROCESSING` items.

- [x] **Step 1: Write test reproducing stranded `PROCESSING` item not being reclaimed**
- [x] **Step 2: Run test to verify it fails**
- [x] **Step 3: Update `claim_pending_webhook_outbox` in `in_memory.rs` and `postgres.rs` to set lease expiry and reclaim expired processing items**
- [x] **Step 4: Run test to verify it passes**
- [x] **Step 5: Verify all outbox tests pass**

---

### Task 5: Workflow Scheduler Starvation Elimination & Config Name Alignment

**Files:**
- Modify: `github-alternative/backend/apps/spine-api/src/db/traits.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/in_memory.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/db/postgres.rs`
- Modify: `github-alternative/backend/apps/spine-api/src/workflow/scheduler.rs`
- Modify: `docs/CORE_FORGE_DEPLOYMENT_PROFILE.md`
- Test: `github-alternative/backend/apps/spine-api/tests/workflow_starvation_prevention_test.rs`

**Interfaces:**
- Consumes: `list_active_workflow_runs_for_repo(repo_id)`
- Produces: FIFO active run ordering without starvation; dual `SPINE_DB_*` / `DATABASE_*` configuration support.

- [x] **Step 1: Write test reproducing starvation when > 100 newer runs exist**
- [x] **Step 2: Run test to verify it fails**
- [x] **Step 3: Implement `list_active_workflow_runs_for_repo` and update scheduler to use it; update `postgres.rs` to read `SPINE_DB_*` variables**
- [x] **Step 4: Run test to verify it passes**
- [x] **Step 5: Verify all scheduler and pool tests pass**

---

### Task 6: Clean Commit, Full Regression Run & Release Artifact Packaging

**Files:**
- Modify: `RELEASE_CANDIDATE_MANIFEST.md`
- Create Git commit for all remediation changes
- Build release candidate binaries (`spine-api`, `spine-coordinator`, `spine-storage-node`, `spine`)

- [x] **Step 1: Run full workspace test suites (`cargo test`)**
- [x] **Step 2: Commit all changes with clean commit message**
- [x] **Step 3: Build release binaries and compute SHA-256 checksums**
- [x] **Step 4: Update `RELEASE_CANDIDATE_MANIFEST.md` to reflect `v0.1.0-rc3` with new checksums and commit hash**
- [x] **Step 5: Verify all artifacts and records match**
