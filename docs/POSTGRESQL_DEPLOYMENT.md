# Spine PostgreSQL Deployment & Schema Migration Guide

## 1. Overview & Requirements

Spine Forge uses PostgreSQL 15+ (recommended: PostgreSQL 16) for all relational control plane data:
- Identity, authentication tokens, sessions, SSH keys.
- Organizations, teams, memberships, and RBAC permissions.
- Repository metadata, branches, tags, protected branch rules.
- Issues, PRs, review comments, review requests.
- Outbound webhooks, deliveries, retry queues.
- Security advisories, dependency audits, SBOM components, secret leaks.
- Discussion categories, threads, and comments.

---

## 2. Sequential Migration Files

All database schema definitions are managed via 11 sequential, idempotent SQL migration files compiled directly into the `spine-api` binary:

1. `0001_core_identity.sql`: Users, sessions, email tokens, password reset tokens, audit events.
2. `0002_organizations_and_teams.sql`: Organizations, memberships, teams, team repositories.
3. `0003_repositories_and_vcs.sql`: Repositories, collaborators, branch protection, SSH deploy keys, user public SSH keys.
4. `0004_issues_and_pull_requests.sql`: Issues, comments, labels, milestones, pull requests, reviews.
5. `0005_notifications_and_events.sql`: User notifications and audit event queries.
6. `0006_artifacts_and_lfs.sql`: Artifact metadata, retention policies, LFS objects, LFS locks, lock audits.
7. `0007_releases_and_packages.sql`: Releases, release assets, packages, package versions.
8. `0008_workflows_and_runners.sql`: Workflow definitions, runs, jobs, runners.
9. `0009_webhooks.sql`: Webhook configurations, events, delivery outbox.
10. `0010_discussions.sql`: Discussion categories, discussions, comments, reactions, subscriptions.
11. `0011_security_center.sql`: Security advisories, dependency vulnerabilities, SBOMs, secret scan reports.

Tracking table: `_schema_migrations (version VARCHAR PRIMARY KEY, applied_at_nanos BIGINT NOT NULL)`.

---

## 3. Migration Execution

### Option A: Automatic at Startup
When `spine-api serve --database-url <url>` boots, it automatically calls `PgDatabase::connect_and_migrate(&url)`, creating `_schema_migrations` if missing and executing all unapplied migrations sequentially inside a transaction.

### Option B: Standalone Migration Command
In continuous deployment pipelines or pre-flight initialization jobs:
```bash
spine-api migrate --database-url "postgres://spine_app:secret@postgres.internal:5432/spine_forge"
```

---

## 4. Current Execution Status & Blocker Reconciliation

- **Implementation Status:** 100% complete. All 11 SQL migration files, DDL statements, indices, foreign keys, and Rust embedded runner logic are written, reviewed, and compiled into the release binary.
- **Verification Blocker:** In the current Windows test workstation, the Docker daemon is offline and PostgreSQL is not installed locally. Per Batch 19 directives, live execution against an active PostgreSQL instance remains marked **BLOCKED** in `docs/BATCH_19_DEPLOYMENT_BLOCKER_REGISTER.md` until executed in an environment with an active PostgreSQL daemon.
