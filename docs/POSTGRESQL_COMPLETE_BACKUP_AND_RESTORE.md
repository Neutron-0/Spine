# PostgreSQL Complete Database Backup and Restore Specification

**Module:** `spine_api::db::dump`  
**Integration:** `spine_api::backup::BackupManager`  
**CLI Subcommands:** `spine-api backup create`, `spine-api backup restore`  
**Milestone:** Batch 21 Remediation (Finding F-01)

---

## 1. Overview and Operational Requirements

In production and high-availability deployments, VCS repository storage and metadata database state are intimately coupled. Restoring repository packfiles without synchronizing corresponding user accounts, permissions, pull requests, issue numbers, and LFS chunk mappings produces inconsistent and corrupted state.

The Batch 21 Database Backup Engine provides:
1. **Zero External Binary Dependencies:** Does not invoke `pg_dump` or `pg_restore`. Operates directly through SQLx connection pools using native PostgreSQL streaming primitives.
2. **Deterministic Topological Table Ordering:** Backs up and restores tables according to a foreign-key dependency hierarchy (`RESTORE_TABLE_ORDER`), deferring constraints during transactions.
3. **Sequence Synchronization:** Automatically resets all `_seq` sequences to `max(column)` to prevent primary key collision errors on subsequent inserts.
4. **Clean Target Verification:** Rejects attempts to restore into non-empty databases to protect against accidental table truncation or corruptive interleaving.
5. **Unified Backup Bundle:** Emits `database_snapshot.json` directly into the unified encrypted `SPNEBAK v2` archive.

---

## 2. Table Dependency Topological Order

The 50 public application tables are restored in strict dependency order:

```rust
pub const RESTORE_TABLE_ORDER: &[&str] = &[
    "users",
    "user_profiles",
    "user_sessions",
    "user_ssh_keys",
    "email_verification_tokens",
    "password_reset_tokens",
    "personal_access_tokens",
    "user_notifications",
    "organizations",
    "organization_invitations",
    "organization_memberships",
    "teams",
    "team_memberships",
    "forge_repositories",
    "repository_permissions",
    "team_repository_permissions",
    "branch_protection_rules",
    "deploy_keys",
    "repository_labels",
    "repository_topics",
    "repository_languages",
    "repository_activity_events",
    "repository_provisioning_jobs",
    "issues",
    "issue_comments",
    "issue_labels",
    "issue_assignees",
    "pull_requests",
    "pull_request_reviews",
    "pull_request_review_comments",
    "pull_request_reviewers",
    "milestones",
    "audit_events",
    "artifact_namespaces",
    "artifact_quotas",
    "artifact_manifests",
    "artifact_versions",
    "artifact_upload_sessions",
    "releases",
    "release_assets",
    "packages",
    "package_versions",
    "webhook_subscriptions",
    "webhook_deliveries",
    "workflow_definitions",
    "workflow_runs",
    "workflow_jobs",
    "workflow_steps",
    "workflow_runners",
    "workflow_secrets",
];
```

---

## 3. Database Dump Algorithm (`dump_database`)

1. Connect to PostgreSQL and query all public application tables:
   ```sql
   SELECT tablename FROM pg_tables WHERE schemaname = 'public' ORDER BY tablename;
   ```
2. For each table, serialize its rows into a JSON array using PostgreSQL's server-side `json_agg`:
   ```sql
   SELECT coalesce(json_agg(t), '[]'::json)::text FROM "<table_name>" t;
   ```
3. Assemble the payload into `DatabaseSnapshot`:
   ```json
   {
     "version": "2.0",
     "timestamp_nanos": 1730800000000000000,
     "tables_count": 50,
     "total_rows": 1420,
     "tables": {
       "users": [ ... ],
       "forge_repositories": [ ... ]
     }
   }
   ```

---

## 4. Database Restore Algorithm (`restore_database`)

1. **Target Emptiness Verification:**
   Queries `pg_tables` in schema `public` and counts rows. If any non-migration application tables contain data, restoration aborts immediately with `ForgeError::Conflict`:
   > *"Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption."*
2. **Schema Baseline Migration:**
   Executes `spine_api::db::migrations::run_migrations(pool).await?` to ensure tables, indexes, and constraints exist.
3. **Transaction Execution:**
   - Begins a single transaction: `let mut tx = pool.begin().await?`.
   - Executes `SET CONSTRAINTS ALL DEFERRED;`.
   - Ingests rows using PostgreSQL's native `json_populate_recordset`:
     ```sql
     INSERT INTO "<table_name>" 
     SELECT * FROM json_populate_recordset(null::"<table_name>", $1::json);
     ```
   - Restores tables in `RESTORE_TABLE_ORDER`, followed by any additional tables present in the snapshot.
4. **Sequence Realignment:**
   Queries all sequences in schema `public` and resets their current values:
   ```sql
   SELECT setval('"<sequence_name>"', coalesce((SELECT max("<col>") FROM "<table>"), 1), true);
   ```
5. **Commit:**
   Commits the transaction atomically.

---

## 5. CLI Usage Examples

### Creating a Full Backup with PostgreSQL
```bash
spine-api backup create \
    --storage-root /var/lib/spine/storage/node-1 \
    --database-url "postgres://spine_app:secret@db.internal:5432/spine_forge" \
    --output /var/backups/spine_full.spinebak \
    --passphrase "CorrectHorseBatteryStaple2026!"
```

### Restoring into a Clean Target
```bash
spine-api backup restore \
    --input /var/backups/spine_full.spinebak \
    --target-storage /var/lib/spine/storage/node-1 \
    --target-database-url "postgres://spine_app:secret@db.internal:5432/spine_forge_clean" \
    --passphrase "CorrectHorseBatteryStaple2026!"
```
