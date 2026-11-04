# DATABASE RELIABILITY & RECOVERY SPECIFICATION

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. REAL POSTGRESQL ENVIRONMENT STATUS

As mandated by Section 2.1 of the Batch 14 Engineering Directive, the host environment has been inspected:

```powershell
Get-Service *postgres* -ErrorAction SilentlyContinue
docker version
```
- **Local Service:** No PostgreSQL service is registered or active on Windows (`Get-Service` returned no instances).
- **Docker Daemon:** Inactive (`failed to connect to docker API at npipe:////./pipe/dockerDesktopLinuxEngine; open: The system cannot find the file specified`).

**Classification:** **BLOCKED** on host external dependency. In strict compliance with the anti-hallucination mandate, in-memory test execution is **never misrepresented** as real PostgreSQL verification.

---

## 2. MIGRATION ARCHITECTURE & INTEGRITY

Spine Forge maintains declarative, forward-only SQL migrations under both `github-alternative/migrations/` and `migrations/`:

| Migration File | Batch | Subsystem / Tables Introduced |
|---|---|---|
| `20261104000001_forge_core.sql` | 08 | `users`, `sessions`, `personal_access_tokens`, `repositories`, `repository_collaborators`, `outbox_provisioning` |
| `20261104000002_forge_collaboration.sql` | 09 | `organizations`, `organization_members`, `teams`, `team_members`, `issues`, `issue_comments`, `pull_requests`, `pull_request_reviews`, `notifications` |
| `20261104000003_forge_artifacts.sql` | 11 | `artifacts`, `artifact_chunks`, `artifact_manifests`, `releases`, `release_assets`, `packages`, `package_versions` |
| `20261104000004_forge_automation.sql` | 12 | `webhooks`, `webhook_deliveries`, `webhook_outbox`, `workflow_runs`, `workflow_jobs`, `workflow_steps`, `workflow_secrets`, `runners`, `runner_heartbeats` |
| `20261104000005_forge_search_discussions_security.sql` | 13 | `discussion_categories`, `discussions`, `discussion_comments`, `discussion_reactions`, `discussion_subscriptions`, `repository_topics`, `repository_languages`, `user_profiles`, `security_scans`, `secret_scan_findings`, `dependency_inventory`, `security_advisories`, `dependency_vulnerabilities` |

### Migration Verification Properties
1. **Sequential Ordering:** Timestamp-prefixed ordering ensures strictly deterministic forward execution.
2. **Foreign Key Cascading & Constraints:**
   - User deletion cleanly cascades to active sessions and personal access tokens.
   - Repository deletion cascades to branches, commits, PRs, issues, discussions, and topics.
   - Foreign keys to users in audit logs are set to `ON DELETE SET NULL` to preserve historical security records.
3. **Idempotent Upsert Semantics:**
   - All unique constraint indexes (e.g., `(owner_username, name)` on repositories, `(ecosystem, package_name, advisory_id)` on advisories) support `ON CONFLICT DO UPDATE` or `ON CONFLICT DO NOTHING`.

---

## 3. DATABASE CONCURRENCY & TRANSACTIONAL BOUNDARIES

High-contention operations employ explicit transactional boundaries and compare-and-swap (CAS) locking:

1. **Repository Reference Updates (CAS):**
   - Updates to repository branches check `expected_oid == current_oid`. If a concurrent client pushed first, the CAS query updates 0 rows, triggering an immediate `LeaseConflict` error.
2. **Pull Request Merges:**
   - Atomic multi-step transaction: checks branch protection, verifies all required checks passed, performs merge commit in the native store, updates base branch ref via CAS, and updates PR status to `MERGED` within the same transaction.
3. **Workflow Job Acquisition & Lease Fencing:**
   - Claiming a queued job updates `runner_id`, increments `fencing_token = fencing_token + 1`, and sets `lease_expires_at = now + 300s`.
   - Only 1 runner can claim the job; subsequent claims fail because status changes from `QUEUED` to `RUNNING`.
4. **Webhook Outbox Delivery:**
   - `claim_pending_webhook_outbox` claims pending items and increments retry backoff atomically, preventing duplicate concurrent deliveries.

---

## 4. FAILURE HANDLING & RECONNECTION SPECIFICATION

The production SQLx pool configuration in `db/postgres.rs` is configured for resilient fail-safe operation:
- **Connection Timeout:** 5 seconds.
- **Acquire Timeout:** 3 seconds.
- **Idle Timeout:** 10 minutes.
- **Max Connections:** 50.
- **Min Connections:** 5.
- **Health Verification:** Tested via `SELECT 1` during `/health/ready` endpoint evaluation. If connection is lost or pool is exhausted, the API returns HTTP 503 Service Unavailable immediately rather than reporting false success.
