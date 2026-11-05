use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;

/// Tables in topological insertion order for foreign-key constraint compliance during restore.
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
    "webhook_outbox",
    "webhook_deliveries",
    "workflow_definitions",
    "workflow_runs",
    "workflow_jobs",
    "workflow_steps",
    "workflow_logs",
    "workflow_secrets",
    "workflow_check_results",
    "runner_registrations",
    "discussion_categories",
    "discussions",
    "discussion_comments",
    "discussion_reactions",
    "discussion_subscriptions",
    "security_advisories",
    "security_scans",
    "secret_scan_findings",
    "dependency_inventories",
    "dependency_vulnerabilities",
    "lfs_locks",
    "lfs_lock_audits",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    pub version: String,
    pub timestamp_nanos: i64,
    pub tables_count: usize,
    pub total_rows: usize,
    pub tables: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseRestoreReport {
    pub tables_restored: usize,
    pub rows_restored: usize,
    pub duration_ms: u64,
}

/// Check if the target database is clean and empty of public application tables.
pub async fn is_database_empty(pool: &PgPool) -> Result<bool, ForgeError> {
    let row: (i64,) = sqlx::query_as(
        r#"
        SELECT COUNT(*) 
        FROM pg_tables 
        WHERE schemaname = 'public' AND tablename != '_schema_migrations'
        "#,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| {
        ForgeError::Internal(format!("Failed to inspect target database tables: {}", e))
    })?;

    Ok(row.0 == 0)
}

/// Dumps all PostgreSQL tables into a structured JSON snapshot.
pub async fn dump_database(pool: &PgPool) -> Result<String, ForgeError> {
    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // List all public application tables
    let rows = sqlx::query(
        r#"
        SELECT tablename 
        FROM pg_tables 
        WHERE schemaname = 'public' 
        ORDER BY tablename
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| ForgeError::Internal(format!("Failed to list database tables: {}", e)))?;

    let mut tables = BTreeMap::new();
    let mut total_rows = 0usize;

    for r in rows {
        let table_name: String = r.get("tablename");

        // Query all rows as JSON array using PostgreSQL's json_agg
        let query_sql = format!(
            "SELECT coalesce(json_agg(t), '[]'::json)::text FROM \"{}\" t",
            table_name
        );
        let agg_row = sqlx::query(&query_sql).fetch_one(pool).await.map_err(|e| {
            ForgeError::Internal(format!("Failed to dump table '{}': {}", table_name, e))
        })?;

        let json_text: String = agg_row.get(0);
        let parsed: serde_json::Value =
            serde_json::from_str(&json_text).unwrap_or(serde_json::Value::Array(vec![]));

        if let serde_json::Value::Array(ref arr) = parsed {
            total_rows += arr.len();
        }

        tables.insert(table_name, parsed);
    }

    let snapshot = DatabaseSnapshot {
        version: "2.0".to_string(),
        timestamp_nanos: now_nanos,
        tables_count: tables.len(),
        total_rows,
        tables,
    };

    serde_json::to_string_pretty(&snapshot)
        .map_err(|e| ForgeError::Internal(format!("Failed to serialize database snapshot: {}", e)))
}

/// Restores a database snapshot into a clean target database, verifying it is empty first.
pub async fn restore_database(
    pool: &PgPool,
    snapshot_json: &str,
) -> Result<DatabaseRestoreReport, ForgeError> {
    let start = std::time::Instant::now();

    // 1. Mandatory Empty Target Check (Audit finding F-01 requirement)
    let empty = is_database_empty(pool).await?;
    if !empty {
        return Err(ForgeError::Conflict(
            "Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption.".to_string(),
        ));
    }

    // 2. Establish schema by applying standard migrations
    tracing::info!("Applying baseline migrations to target database prior to data restore...");
    crate::db::migrations::run_migrations(pool).await?;

    let snapshot: DatabaseSnapshot = serde_json::from_str(snapshot_json).map_err(|e| {
        ForgeError::Internal(format!("Failed to parse database snapshot JSON: {}", e))
    })?;

    // 3. Begin single restore transaction with deferred constraints
    let mut tx = pool.begin().await.map_err(|e| {
        ForgeError::Internal(format!(
            "Failed to begin database restore transaction: {}",
            e
        ))
    })?;

    let _ = sqlx::query("SET CONSTRAINTS ALL DEFERRED")
        .execute(&mut *tx)
        .await;

    let mut tables_restored = 0usize;
    let mut rows_restored = 0usize;

    // First restore in topological order
    for table in RESTORE_TABLE_ORDER {
        if let Some(data) = snapshot.tables.get(*table) {
            if let serde_json::Value::Array(ref arr) = data {
                if !arr.is_empty() {
                    let insert_sql = format!(
                        "INSERT INTO \"{}\" SELECT * FROM json_populate_recordset(null::\"{}\", $1::json)",
                        table, table
                    );
                    sqlx::query(&insert_sql)
                        .bind(data)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            ForgeError::Internal(format!(
                                "Failed to insert restored rows into '{}': {}",
                                table, e
                            ))
                        })?;

                    tables_restored += 1;
                    rows_restored += arr.len();
                }
            }
        }
    }

    // Next restore any additional tables in snapshot that were not in RESTORE_TABLE_ORDER (e.g. future migrations)
    for (table, data) in &snapshot.tables {
        if !RESTORE_TABLE_ORDER.contains(&table.as_str()) && table != "_schema_migrations" {
            if let serde_json::Value::Array(ref arr) = data {
                if !arr.is_empty() {
                    let insert_sql = format!(
                        "INSERT INTO \"{}\" SELECT * FROM json_populate_recordset(null::\"{}\", $1::json)",
                        table, table
                    );
                    let _ = sqlx::query(&insert_sql).bind(data).execute(&mut *tx).await;
                    tables_restored += 1;
                    rows_restored += arr.len();
                }
            }
        }
    }

    // 4. Synchronize all sequences to max values
    let seq_rows = sqlx::query(
        r#"
        SELECT sequence_name 
        FROM information_schema.sequences 
        WHERE sequence_schema = 'public'
        "#,
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();

    for row in seq_rows {
        let seq_name: String = row.get("sequence_name");
        // E.g. issues_number_seq -> table issues, column number
        if let Some(table_col) = seq_name.strip_suffix("_seq") {
            if let Some((table, col)) = table_col.split_once('_') {
                let reset_sql = format!(
                    "SELECT setval('\"{}\"', coalesce((SELECT max(\"{}\") FROM \"{}\"), 1), true)",
                    seq_name, col, table
                );
                let _ = sqlx::query(&reset_sql).execute(&mut *tx).await;
            }
        }
    }

    tx.commit().await.map_err(|e| {
        ForgeError::Internal(format!(
            "Failed to commit database restore transaction: {}",
            e
        ))
    })?;

    let duration_ms = start.elapsed().as_millis() as u64;
    tracing::info!(
        "Database restore completed: {} tables, {} rows in {} ms",
        tables_restored,
        rows_restored,
        duration_ms
    );

    Ok(DatabaseRestoreReport {
        tables_restored,
        rows_restored,
        duration_ms,
    })
}
