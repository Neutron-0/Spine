# API AUTHORIZATION & TENANT-ISOLATION MATRIX

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. ACCESS CONTROL MATRIX

| API Resource / Endpoint | Anonymous User | Authenticated Non-Member | Repo Reader (`READ`) | Repo Writer (`WRITE`) | Repo Admin (`ADMIN`) | Org Admin | Registered Runner |
|---|---|---|---|---|---|---|---|
| **Public Repo (Read / Clone)** | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ (Needs token) |
| **Private Repo (Read / Clone)** | ❌ 401 Unauthorized | ❌ 403 / 404 Hidden | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ (Needs ticket) |
| **Branch Push / Update** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (if unprotected) | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Protected Branch Push** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Blocked (Needs PR) | ❌ (Needs PR/Admin) | ❌ (Needs PR) | ❌ 403 Forbidden |
| **Pull Request (Open / Comment)**| ❌ 401 Unauthorized | ✅ Permitted (Public) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Pull Request (Merge)** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Checks pass) | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Discussions (Read)** | ✅ Permitted (Public) | ✅ Permitted (Public) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Discussions (Create / Reply)**| ❌ 401 Unauthorized | ✅ Permitted (Public) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Mark Accepted Answer** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 (Unless OP) | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **LFS Upload / Download** | Download only (Public) | Download only (Public) | Download only | ✅ Read / Write | ✅ Read / Write | ✅ Read / Write | ❌ 403 Forbidden |
| **Cargo Package Publish** | ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |
| **Workflow Run (Trigger / View)**| View only (Public) | View only (Public) | View only | ✅ Trigger / View | ✅ Trigger / View | ✅ Trigger / View | ❌ 403 Forbidden |
| **Workflow Job (Acquire / Log)**| ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted (Scoped lease) |
| **Workflow Secrets (View / Set)**| ❌ 401 Unauthorized | ❌ 403 Forbidden | ❌ 403 Forbidden | ❌ 403 Forbidden | ✅ Permitted | ✅ Permitted | ❌ (Masked in logs) |
| **Security Center (Findings / Scans)**| ❌ 401 Unauthorized | ❌ 403 / 404 Hidden | ✅ View only | ✅ Scan / Remediate | ✅ Full Admin | ✅ Full Admin | ❌ 403 Forbidden |
| **SBOM Export (SPDX / CycloneDX)**| ❌ 401 Unauthorized | ❌ 403 / 404 Hidden | ✅ Permitted | ✅ Permitted | ✅ Permitted | ✅ Permitted | ❌ 403 Forbidden |

---

## 2. HORIZONTAL PRIVILEGE ESCALATION PREVENTION

1. **Existence Obfuscation:** Unauthorized attempts to access private repositories return HTTP 404 Not Found rather than HTTP 403 Forbidden, preventing attackers from probing private repository existence.
2. **Tenant ID Scoping:** All database queries for issues, PRs, comments, artifacts, releases, and workflow jobs include `WHERE repo_id = $1` in their primary query filters. A client cannot access another repository's issue or secret by manipulating integers or UUIDs.
3. **Fencing Tokens:** Workflow runner updates are bound to `(job_id, runner_id, fencing_token)`. One runner cannot report progress on another runner's active lease.
