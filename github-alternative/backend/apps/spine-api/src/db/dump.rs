use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::BTreeMap;
use std::io::BufRead;

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
pub struct SequenceEntry {
    pub name: String,
    pub last_value: i64,
    pub is_called: bool,
}

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
/// Emits streaming JSON Lines format (version 3.0) preserving arbitrary numeric precision byte-for-byte.
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

    // 2. Query applied migrations for schema compatibility verification
    let migration_rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT version, applied_at_nanos FROM _schema_migrations ORDER BY version")
            .fetch_all(&mut *tx)
            .await
            .unwrap_or_default();

    // 3. Query current sequence states
    let seq_state_rows = sqlx::query(
        "SELECT sequencename, last_value FROM pg_sequences WHERE schemaname = 'public'",
    )
    .fetch_all(&mut *tx)
    .await
    .unwrap_or_default();

    let mut recorded_sequences = Vec::new();
    for row in seq_state_rows {
        let name: String = row.get("sequencename");
        let last_value: Option<i64> = row.try_get("last_value").ok();
        recorded_sequences.push(SequenceEntry {
            name,
            last_value: last_value.unwrap_or(1),
            is_called: last_value.is_some(),
        });
    }

    // 4. Discover all public application tables
    let table_rows = sqlx::query(
        r#"
        SELECT tablename 
        FROM pg_tables 
        WHERE schemaname = 'public' 
          AND tablename != '_cluster_maintenance_barrier'
          AND tablename != '_schema_migrations'
        ORDER BY tablename
        "#,
    )
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| ForgeError::Internal(format!("Failed to list database tables: {}", e)))?;

    let existing_tables: std::collections::HashSet<String> = table_rows
        .into_iter()
        .map(|r| r.get::<String, _>("tablename"))
        .collect();

    // Order tables by RESTORE_TABLE_ORDER first, then any unexpected/extra tables
    let mut ordered_tables = Vec::new();
    for tbl in RESTORE_TABLE_ORDER {
        if existing_tables.contains(*tbl) {
            ordered_tables.push(tbl.to_string());
        }
    }
    for tbl in &existing_tables {
        if !ordered_tables.contains(tbl) {
            ordered_tables.push(tbl.clone());
        }
    }

    // Write Header record (JSON Line 1)
    let header_json = serde_json::json!({
        "record": "header",
        "version": "3.0",
        "timestamp_nanos": now_nanos,
        "tables_count": ordered_tables.len(),
        "applied_migrations": migration_rows,
        "sequences": recorded_sequences,
    });
    writer
        .write_all(header_json.to_string().as_bytes())
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    writer
        .write_all(b"\n")
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    let mut total_rows = 0usize;

    // 5. Stream table rows directly to writer with zero intermediate float parsing
    use futures_util::StreamExt;
    for table_name in &ordered_tables {
        let start_json = serde_json::json!({
            "record": "table_start",
            "table": table_name,
        });
        writer
            .write_all(start_json.to_string().as_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        writer
            .write_all(b"\n")
            .map_err(|e| ForgeError::Internal(e.to_string()))?;

        let query_sql = format!(
            "SELECT row_to_json(t)::text AS row_json FROM \"{}\" t",
            table_name
        );
        let mut row_stream = sqlx::query(&query_sql).fetch(&mut *tx);

        let mut table_rows_count = 0usize;
        while let Some(row_res) = row_stream.next().await {
            let row = row_res.map_err(|e| {
                ForgeError::Internal(format!("Failed to read row from '{}': {}", table_name, e))
            })?;
            let row_json: String = row.get("row_json");

            // Direct line write: preserves exact numeric tokens and JSONB structure
            writer
                .write_all(b"{\"record\":\"row\",\"table\":\"")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            writer
                .write_all(table_name.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            writer
                .write_all(b"\",\"data\":")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            writer
                .write_all(row_json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            writer
                .write_all(b"}\n")
                .map_err(|e| ForgeError::Internal(e.to_string()))?;

            table_rows_count += 1;
            total_rows += 1;
        }

        let end_json = serde_json::json!({
            "record": "table_end",
            "table": table_name,
            "row_count": table_rows_count,
        });
        writer
            .write_all(end_json.to_string().as_bytes())
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        writer
            .write_all(b"\n")
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
    }

    // Write Footer record
    let footer_json = serde_json::json!({
        "record": "footer",
        "total_tables": ordered_tables.len(),
        "total_rows": total_rows,
    });
    writer
        .write_all(footer_json.to_string().as_bytes())
        .map_err(|e| ForgeError::Internal(e.to_string()))?;
    writer
        .write_all(b"\n")
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    writer
        .flush()
        .map_err(|e| ForgeError::Internal(e.to_string()))?;

    let _ = tx.commit().await;

    Ok(())
}

/// Restores a database snapshot from a streaming reader into a clean, empty target database.
/// Strictly bounded in memory O(batch_size) and preserves exact arbitrary numeric precision.
pub async fn restore_database_from_reader<R: BufRead>(
    pool: &PgPool,
    mut reader: R,
) -> Result<DatabaseRestoreReport, ForgeError> {
    let start = std::time::Instant::now();

    // 1. Mandatory Empty Target Check (Audit finding F-01 requirement)
    let empty = is_database_empty(pool).await?;
    if !empty {
        return Err(ForgeError::Conflict(
            "Target database is not empty. Full database restoration requires a clean, empty target database to prevent data corruption.".to_string(),
        ));
    }

    // 2. Read first line to detect format and validate header
    let mut first_line = String::new();
    reader
        .read_line(&mut first_line)
        .map_err(|e| ForgeError::Internal(format!("Failed to read snapshot header line: {}", e)))?;

    if first_line.trim().is_empty() {
        return Err(ForgeError::Internal(
            "Database snapshot is empty".to_string(),
        ));
    }

    // If legacy version 2.0 format (whole JSON document), route to legacy parser
    if first_line
        .trim_start()
        .starts_with("{\n  \"version\": \"2.0\"")
        || first_line.contains("\"tables\": {")
    {
        let mut rest = String::new();
        reader
            .read_to_string(&mut rest)
            .map_err(|e| ForgeError::Internal(e.to_string()))?;
        let full_json = format!("{}{}", first_line, rest);
        return restore_database_legacy_v2(pool, &full_json).await;
    }

    // Parse streaming JSON Lines header
    let header_val: serde_json::Value = serde_json::from_str(&first_line)
        .map_err(|e| ForgeError::Internal(format!("Failed to parse snapshot header: {}", e)))?;

    let version = header_val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    if version != "3.0" {
        return Err(ForgeError::Internal(format!(
            "Unsupported database snapshot version: '{}'. Expected '3.0'.",
            version
        )));
    }

    // 3. Establish schema by applying standard migrations
    tracing::info!("Applying baseline migrations to target database prior to data restore...");
    crate::db::migrations::run_migrations(pool).await?;

    // 4. Begin single restore transaction with deferred constraints
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

    // Restore recorded _schema_migrations if available in header
    if let Some(migrations) = header_val
        .get("applied_migrations")
        .and_then(|m| m.as_array())
    {
        for m in migrations {
            if let Some(arr) = m.as_array() {
                if arr.len() == 2 {
                    if let (Some(ver), Some(ts)) = (arr[0].as_str(), arr[1].as_i64()) {
                        let _ = sqlx::query(
                            "INSERT INTO _schema_migrations (version, applied_at_nanos) VALUES ($1, $2) ON CONFLICT (version) DO UPDATE SET applied_at_nanos = $2"
                        )
                        .bind(ver)
                        .bind(ts)
                        .execute(&mut *tx)
                        .await;
                    }
                }
            }
        }
    }

    let mut tables_restored = 0usize;
    let mut rows_restored = 0usize;
    let mut current_table: Option<String> = None;
    let mut row_batch: Vec<String> = Vec::with_capacity(500);

    let mut line = String::new();
    while reader
        .read_line(&mut line)
        .map_err(|e| ForgeError::Internal(format!("Failed to read line from snapshot: {}", e)))?
        > 0
    {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            line.clear();
            continue;
        }

        let record_val: serde_json::Value = serde_json::from_str(trimmed)
            .map_err(|e| ForgeError::Internal(format!("Malformed JSONL record: {}", e)))?;

        let record_type = record_val
            .get("record")
            .and_then(|r| r.as_str())
            .unwrap_or("");

        match record_type {
            "table_start" => {
                let table_name = record_val
                    .get("table")
                    .and_then(|t| t.as_str())
                    .ok_or_else(|| {
                        ForgeError::Internal("Missing table name in table_start".to_string())
                    })?
                    .to_string();
                current_table = Some(table_name);
                row_batch.clear();
            }
            "row" => {
                let table_name = record_val
                    .get("table")
                    .and_then(|t| t.as_str())
                    .unwrap_or("");
                if let Some(cur) = &current_table {
                    if cur != table_name {
                        return Err(ForgeError::Internal(format!(
                            "Row record table '{}' does not match active table '{}'",
                            table_name, cur
                        )));
                    }
                }

                // Extract row data JSON text directly preserving arbitrary numeric precision
                if let Some(data) = record_val.get("data") {
                    row_batch.push(data.to_string());
                }

                if row_batch.len() >= 500 {
                    if let Some(table) = &current_table {
                        insert_row_batch(&mut tx, table, &row_batch).await?;
                        rows_restored += row_batch.len();
                        row_batch.clear();
                    }
                }
            }
            "table_end" => {
                if let Some(table) = &current_table {
                    if !row_batch.is_empty() {
                        insert_row_batch(&mut tx, table, &row_batch).await?;
                        rows_restored += row_batch.len();
                        row_batch.clear();
                    }
                    tables_restored += 1;
                }
                current_table = None;
            }
            "footer" => {
                break;
            }
            _ => {}
        }

        line.clear();
    }

    // 5. Sequence Resynchronization:
    // First, restore sequences linked to table columns via system catalog
    let seq_rows = sqlx::query(
        r#"
        SELECT
            s.relname AS sequence_name,
            t.relname AS table_name,
            a.attname AS column_name
        FROM pg_class s
        JOIN pg_depend d ON d.objid = s.oid AND d.classid = 'pg_class'::regclass AND d.deptype IN ('a', 'i')
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

    // Second, restore recorded sequence states from header for any standalone sequences
    if let Some(recorded_seqs) = header_val.get("sequences").and_then(|s| s.as_array()) {
        for s in recorded_seqs {
            if let (Some(name), Some(last_val), Some(is_called)) = (
                s.get("name").and_then(|n| n.as_str()),
                s.get("last_value").and_then(|v| v.as_i64()),
                s.get("is_called").and_then(|c| c.as_bool()),
            ) {
                let _ = sqlx::query(&format!("SELECT setval('\"{}\"', $1, $2)", name))
                    .bind(last_val)
                    .bind(is_called)
                    .execute(&mut *tx)
                    .await;
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

/// Helper to insert a batch of JSON rows into a table via json_populate_recordset
async fn insert_row_batch(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    table: &str,
    rows: &[String],
) -> Result<(), ForgeError> {
    if rows.is_empty() {
        return Ok(());
    }

    let batch_json = format!("[{}]", rows.join(","));
    let insert_sql = format!(
        "INSERT INTO \"{}\" SELECT * FROM json_populate_recordset(null::\"{}\", $1::json)",
        table, table
    );

    sqlx::query(&insert_sql)
        .bind(&batch_json)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to insert restored batch of {} rows into '{}': {}",
                rows.len(),
                table,
                e
            ))
        })?;

    Ok(())
}

/// Restores a database snapshot from a JSON string.
/// Supports both version 3.0 streaming format and legacy version 2.0 format.
pub async fn restore_database(
    pool: &PgPool,
    snapshot_json: &str,
) -> Result<DatabaseRestoreReport, ForgeError> {
    restore_database_from_reader(pool, std::io::Cursor::new(snapshot_json.as_bytes())).await
}

/// Legacy restore for Version 2.0 snapshots.
async fn restore_database_legacy_v2(
    pool: &PgPool,
    snapshot_json: &str,
) -> Result<DatabaseRestoreReport, ForgeError> {
    let start = std::time::Instant::now();

    let snapshot: DatabaseSnapshot = serde_json::from_str(snapshot_json).map_err(|e| {
        ForgeError::Internal(format!("Failed to parse database snapshot JSON: {}", e))
    })?;

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

    // Synchronize sequences
    let seq_rows = sqlx::query(
        r#"
        SELECT
            s.relname AS sequence_name,
            t.relname AS table_name,
            a.attname AS column_name
        FROM pg_class s
        JOIN pg_depend d ON d.objid = s.oid AND d.classid = 'pg_class'::regclass AND d.deptype IN ('a', 'i')
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
    Ok(DatabaseRestoreReport {
        tables_restored,
        rows_restored,
        duration_ms,
    })
}
