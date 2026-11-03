# Batch 09 Verification and Test Suite Execution Guide

## 1. Overview

Batch 09 introduces 6 targeted integration test suites covering the collaboration layer in `github-alternative/backend/tests/`, complementing the existing authentication, provisioning, and permissions tests.

All tests run natively on Windows 11 using Rust 1.97.1 MSVC and in-memory or PostgreSQL database adapters.

---

## 2. Test Suites Summary

### Backend Tests (`github-alternative/backend/tests/`)

1. **`org_teams_test.rs`:**
   - Verifies organization creation and slug normalization.
   - Verifies owner, admin, and member role assignment.
   - Tests team creation and member addition.
   - Verifies team repository permission assignment (`ADMIN`, `MAINTAIN`, `WRITE`, `READ`).

2. **`issues_test.rs`:**
   - Verifies issue creation with sequential per-repository issue numbers.
   - Tests issue assignment and label attachment.
   - Tests milestone creation and association with issues.
   - Verifies issue commenting and status transition (`OPEN` -> `CLOSED`).

3. **`pull_requests_test.rs`:**
   - Creates real native Spine repositories with diverging commit histories on `main` and `feature`.
   - Opens a pull request proposing `feature` into `main`.
   - Computes three-dot unified diffs and validates file additions/deletions.
   - Evaluates mergeability report verifying non-conflicting 3-way tree merge cleanly.

4. **`code_review_test.rs`:**
   - Submits formal reviews (`APPROVED`, `CHANGES_REQUESTED`).
   - Posts line-level inline review comments on diff hunks.
   - Resolves inline review comments.
   - Tests reviewer assignment and status updates.

5. **`merge_safety_test.rs`:**
   - Configures branch protection rules requiring PRs, approvals, self-approval prevention, and addressed comments.
   - Verifies that unapproved PRs cannot be merged.
   - Verifies that PRs with unresolved change requests cannot be merged.
   - Submits required approvals from a secondary reviewer.
   - Executes atomic three-way merge, confirming the creation of a 2-parent commit and update of `.spine/refs/heads/main`.
   - Verifies referenced issue auto-closing.

6. **`notifications_test.rs`:**
   - Verifies notification creation for review requests, mentions, and assignments.
   - Queries unread notifications.
   - Marks individual notifications as read.
   - Marks all notifications as read in bulk.

---

## 3. Execution Commands

### Running Backend Collaboration Tests:
```powershell
cargo test --manifest-path github-alternative/backend/Cargo.toml
```
Expected output: **21 passed, 0 failed**.

### Running Native VCS Engine Tests:
```powershell
cargo test --manifest-path git-alternative/Cargo.toml
```
Expected output: **105 passed, 0 failed**.

### Verifying Frontend TypeScript and Vite Build:
```powershell
npm --prefix github-alternative/frontend run build
```
Expected output: **tsc && vite build succeeded with 0 errors**.
