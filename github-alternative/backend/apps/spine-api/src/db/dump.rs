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
        WHERE schemaname = 'public' 
          AND tablename != '_schema_migrations'
          AND tablename != '_cluster_maintenance_barrier'
        "#,
    )
    .fetch_one(pool)
    .await
    .map_err(|e| {
        ForgeError::Internal(format!("Failed to inspect target database tables: {}", e))
    })?;

    Ok(row.0 == 0)
}

/// Dumps all PostgreSQL tables into a structured JSON snapshot string inside a consistent REPEATABLE READ snapshot transaction.
pub async fn dump_database(pool: &PgPool) -> Result<String, ForgeError> {
    let mut buf = Vec::new();
    dump_database_to_writer(pool, &mut buf).await?;
    String::from_utf8(buf)
        .map_err(|e| ForgeError::Internal(format!("Invalid UTF-8 in snapshot: {}", e)))
}

/// Streams database snapshot into an io::Write stream with bounded memory footprint and REPEATABLE READ snapshot isolation.
pub async fn dump_database_to_writer<W: std::io::Write>(
    pool: &PgPool,
    mut writer: W,
) -> Result<(), ForgeError> {
    let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);

    // 1. Begin consistent snapshot transaction
    let mut tx = pool.begin().await.map_err(|e| {
        ForgeError::Internal(format!("Failed to begin database dump transaction: {}", e))
    })?;

    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|e| ForgeError::Internal(format!("Failed to set REPEATABLE READ: {}", e)))?;

    // 2. Discover all public tables
    let rows = sqlx::query(
        r#"
        SELECT tablename 
        FROM pg_tables 
        WHERE schemaname = 'public' 
          AND tablename != '_cluster_maintenance_barrier'
        ORDER BY tablename
        "#,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ForgeError::Internal(format!("Failed to list database tables: {}", e)))?;

    let tables_count = rows.len();
    writer
        .write_all(
            format!(
                "{{\n  \"version\": \"2.0\",\n  \"timestamp_nanos\": {},\n  \"tables_count\": {},\n  \"tables\": {{\n",
                now_nanos, tables_count
            )
            .as_bytes(),
        )
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    let mut total_rows = 0usize;

    for (t_idx, r) in rows.iter().enumerate() {
        let table_name: String = r.get("tablename");

        writer
            .write_all(format!("    \"{}\": [\n", table_name).as_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        // Query rows formatted as JSON lines streaming from PostgreSQL
        let query_sql = format!(
            "SELECT row_to_json(t)::text AS row_json FROM \"{}\" t",
            table_name
        );
        let mut row_stream = sqlx::query(&query_sql).fetch(&mut *tx);

        let mut row_idx = 0usize;
        use futures_util::StreamExt;
        while let Some(row_res) = row_stream.next().await {
            let row = row_res.map_err(|e| {
                ForgeError::Internal(format!("Failed to read row from '{}': {}", table_name, e))
            })?;
            let row_json: String = row.get("row_json");

            if row_idx > 0 {
                writer
                    .write_all(b",\n")
                    .map_err(|e| ForgeError::Internal(e.to_string()))?;
            }
            writer
                .write_all(format!("      {}", row_json).as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;

            row_idx += 1;
            total_rows += 1;
        }

        let is_last_table = t_idx + 1 == tables_count;
        if is_last_table {
            writer
                .write_all(b"\n    ]\n")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        } else {
            writer
                .write_all(b"\n    ],\n")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
    }

    writer
        .write_all(format!("  }},\n  \"total_rows\": {}\n}}\n", total_rows).as_bytes())
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    writer
        .flush()
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    let _ = tx.commit().await;

    Ok(())
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

    sqlx::query("SET CONSTRAINTS ALL DEFERRED")
        .execute(&mut *tx)
        .await
        .map_err(|e| ForgeError::Internal(format!("Failed to defer constraints: {}", e)))?;

    let mut tables_restored = 0usize;
    let mut rows_restored = 0usize;

    // First restore in topological order
    for table in RESTORE_TABLE_ORDER {
        if let Some(serde_json::Value::Array(ref arr)) = snapshot.tables.get(*table) {
            if !arr.is_empty() {
                for chunk in arr.chunks(500) {
                    let chunk_val = serde_json::Value::Array(chunk.to_vec());
                    let insert_sql = format!(
                            "INSERT INTO \"{}\" SELECT * FROM json_populate_recordset(null::\"{}\", $1::json)",
                            table, table
                        );
                    sqlx::query(&insert_sql)
                        .bind(&chunk_val)
                        .execute(&mut *tx)
                        .await
                        .map_err(|e| {
                            ForgeError::Internal(format!(
                                "Failed to insert restored rows into '{}': {}",
                                table, e
                            ))
                        })?;
                }

                tables_restored += 1;
                rows_restored += arr.len();
            }
        }
    }

    // Next restore any additional tables in snapshot that were not in RESTORE_TABLE_ORDER (e.g. future migrations)
    for (table, data) in &snapshot.tables {
        if !RESTORE_TABLE_ORDER.contains(&table.as_str())
            && table != "_schema_migrations"
            && table != "_cluster_maintenance_barrier"
        {
            if let serde_json::Value::Array(ref arr) = data {
                if !arr.is_empty() {
                    for chunk in arr.chunks(500) {
                        let chunk_val = serde_json::Value::Array(chunk.to_vec());
                        let insert_sql = format!(
                            "INSERT INTO \"{}\" SELECT * FROM json_populate_recordset(null::\"{}\", $1::json)",
                            table, table
                        );
                        sqlx::query(&insert_sql)
                            .bind(&chunk_val)
                            .execute(&mut *tx)
                            .await
                            .map_err(|e| {
                                ForgeError::Internal(format!(
                                    "Failed to insert restored rows into '{}': {}",
                                    table, e
                                ))
                            })?;
                    }
                    tables_restored += 1;
                    rows_restored += arr.len();
                }
            }
        }
    }

    // 4. Synchronize all sequences to max values using PostgreSQL system catalog (pg_depend, pg_class, pg_attribute)
    let seq_rows = sqlx::query(
        r#"
        SELECT 
            s.relname AS sequence_name,
            t.relname AS table_name,
            a.attname AS column_name
        FROM pg_class s
        JOIN pg_depend d ON d.objid = s.oid AND d.classid = 'pg_class'::regclass AND d.deptype = 'a'
        JOIN pg_class t ON t.oid = d.refobjid
        JOIN pg_attribute a ON a.attrelid = t.oid AND a.attnum = d.refobjsubid
        WHERE s.relkind = 'S'
        "#,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ForgeError::Internal(format!("Failed to query system sequence catalog: {}", e)))?;

    for row in seq_rows {
        let seq_name: String = row.get("sequence_name");
        let table: String = row.get("table_name");
        let col: String = row.get("column_name");

        let reset_sql = format!(
            "SELECT setval('\"{}\"', coalesce((SELECT max(\"{}\") FROM \"{}\"), 1), true)",
            seq_name, col, table
        );
        sqlx::query(&reset_sql)
            .execute(&mut *tx)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to synchronize sequence '{}' for table '{}.{}': {}",
                    seq_name, table, col, e
                ))
            })?;
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
