# PostgreSQL Schema Parity and Migration Reconciliation

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Scope:** Strict Schema Parity between SQL Migration Definitions (`0001`–`0011`) and `PgDatabase` Object-Relational Mappings (`postgres.rs`).

---

## 1. Overview and Problem Statement

During the live PostgreSQL 18.6 testing phase in Batch 20, runtime query execution revealed subtle field disparities between the initial migration schemas written during early development batches and the hardened query signatures implemented in `github-alternative/backend/apps/spine-api/src/db/postgres.rs`.

Although SQLite allows looser column typing and silent omissions, PostgreSQL enforces strict schema adherence, column existence, nullability constraints, and type constraints (such as `JSONB`).

This document details the exact differences identified and the reconciliation applied to achieve 100% parity across migrations, queries, and domain entities.

---

## 2. Reconciled Disparities

### 2.1. `user_sessions` Table
- **Disparity:** Migration `0001_core_identity.sql` defined columns `(session_token, csrf_token, last_active_nanos)`. However, `postgres.rs` and the `UserSession` domain struct query `(session_token_hash, ip_address, user_agent, created_at_nanos, expires_at_nanos, revoked)`.
- **Resolution:** Reconciled `0001_core_identity.sql`:
  ```sql
  CREATE TABLE IF NOT EXISTS user_sessions (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
      session_token_hash TEXT NOT NULL UNIQUE,
      ip_address TEXT,
      user_agent TEXT,
      created_at_nanos BIGINT NOT NULL,
      expires_at_nanos BIGINT NOT NULL,
      revoked BOOLEAN NOT NULL DEFAULT FALSE
  );
  CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON user_sessions (user_id);
  CREATE INDEX IF NOT EXISTS idx_sessions_token_hash ON user_sessions (session_token_hash);
  ```

### 2.2. `personal_access_tokens` Table
- **Disparity:** `0001_core_identity.sql` specified `scopes_json TEXT NOT NULL`. `postgres.rs` binds `scopes JSONB NOT NULL DEFAULT '[]'::jsonb`.
- **Resolution:** Updated column definition to native PostgreSQL `JSONB`:
  ```sql
  CREATE TABLE IF NOT EXISTS personal_access_tokens (
      id TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
      name TEXT NOT NULL,
      token_hash TEXT NOT NULL UNIQUE,
      scopes JSONB NOT NULL DEFAULT '[]'::jsonb,
      expires_at_nanos BIGINT NOT NULL,
      last_used_at_nanos BIGINT,
      created_at_nanos BIGINT NOT NULL,
      revoked BOOLEAN NOT NULL DEFAULT FALSE
  );
  ```

### 2.3. `email_verification_tokens` & `password_reset_tokens`
- **Disparity 1:** Query `consume_email_token` and `consume_reset_token` executed:
  `UPDATE ... SET used_at_nanos = $1 WHERE token_hash = $2 AND used_at_nanos IS NULL AND expires_at_nanos >= $1 RETURNING user_id`.
  The original migration lacked `used_at_nanos BIGINT`.
- **Disparity 2:** `create_email_token` and `create_reset_token` inserted only `(token_hash, user_id, expires_at_nanos)`. The migration had `created_at_nanos BIGINT NOT NULL` without a default value, triggering a not-null constraint violation on insert.
- **Resolution:** Added `used_at_nanos BIGINT` and supplied default timestamps:
  ```sql
  CREATE TABLE IF NOT EXISTS password_reset_tokens (
      token_hash TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
      expires_at_nanos BIGINT NOT NULL,
      created_at_nanos BIGINT NOT NULL DEFAULT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000000000)::BIGINT,
      used_at_nanos BIGINT
  );
  CREATE TABLE IF NOT EXISTS email_verification_tokens (
      token_hash TEXT PRIMARY KEY,
      user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
      expires_at_nanos BIGINT NOT NULL,
      created_at_nanos BIGINT NOT NULL DEFAULT (EXTRACT(EPOCH FROM clock_timestamp()) * 1000000000)::BIGINT,
      used_at_nanos BIGINT
  );
  ```

### 2.4. `user_profiles` Table
- **Disparity:** `0001_core_identity.sql` defined basic profile columns `(bio, location, website, company, social_links_json)`. However, `postgres.rs` implements `ExtendedUserProfile` querying `(bio, company, location, website_url, twitter_username, profile_readme, pinned_repo_ids, updated_at_nanos)`.
- **Resolution:** Aligned schema with `ExtendedUserProfile` and JSONB representation:
  ```sql
  CREATE TABLE IF NOT EXISTS user_profiles (
      user_id TEXT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
      bio TEXT,
      company TEXT,
      location TEXT,
      website_url TEXT,
      twitter_username TEXT,
      profile_readme TEXT,
      pinned_repo_ids JSONB NOT NULL DEFAULT '[]'::jsonb,
      updated_at_nanos BIGINT NOT NULL
  );
  ```

### 2.5. `audit_events` Table
- **Disparity:** Migration defined `(id TEXT PRIMARY KEY, actor_user_id, event_type, target_type, target_id, metadata_json, ip_address, created_at_nanos)`. `postgres.rs` inserts `(user_id, action, resource_type, resource_id, ip_address, metadata, timestamp_nanos)`.
- **Resolution:** Reconciled to auto-incrementing `BIGSERIAL` with JSONB metadata:
  ```sql
  CREATE TABLE IF NOT EXISTS audit_events (
      id BIGSERIAL PRIMARY KEY,
      user_id TEXT,
      action TEXT NOT NULL,
      resource_type TEXT NOT NULL,
      resource_id TEXT NOT NULL,
      ip_address TEXT,
      metadata JSONB,
      timestamp_nanos BIGINT NOT NULL
  );
  CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_events (timestamp_nanos DESC);
  ```

---

## 3. Parity Matrix Across All 11 Migrations

| Table Group | Migration File | Status | Notes |
|---|---|---|---|
| Core Identity | `0001_core_identity.sql` | Aligned & Verified | Reconciled sessions, tokens, profiles, audit |
| Orgs & Teams | `0002_organizations_and_teams.sql` | 100% Parity | Verified via org creation and membership |
| Repos & VCS | `0003_repositories_and_vcs.sql` | 100% Parity | Verified repo, deploy keys, branch rules |
| Issues & PRs | `0004_issues_and_pull_requests.sql` | 100% Parity | Verified issue numbers, comments, review state |
| Feeds & Events | `0005_notifications_and_events.sql` | 100% Parity | Notifications, feeds, activity indices |
| LFS & Storage | `0006_artifacts_and_lfs.sql` | 100% Parity | Git LFS locks, object pointers, artifacts |
| Releases & Cargo | `0007_releases_and_packages.sql` | 100% Parity | Releases, assets, crates registry |
| Workflows & CI | `0008_workflows_and_runners.sql` | 100% Parity | CI jobs, runs, runner registration tokens |
| Webhooks | `0009_webhooks.sql` | 100% Parity | Webhook delivery logs, secrets, status |
| Discussions | `0010_discussions.sql` | 100% Parity | Discussion categories, threads, answers |
| Security Center | `0011_security_center.sql` | 100% Parity | Scans, vulnerability advisories, findings |

---

## 4. Conclusion

With these reconciliations in place, the PostgreSQL schema is completely synchronized with the domain logic and persistence layer. Fresh database initializations run without manual intervention or runtime errors.
