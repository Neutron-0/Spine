use crate::core::error::ForgeError;
use sqlx::PgPool;

pub const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_core_identity",
        include_str!("../../migrations/0001_core_identity.sql"),
    ),
    (
        "0002_organizations_and_teams",
        include_str!("../../migrations/0002_organizations_and_teams.sql"),
    ),
    (
        "0003_repositories_and_vcs",
        include_str!("../../migrations/0003_repositories_and_vcs.sql"),
    ),
    (
        "0004_issues_and_pull_requests",
        include_str!("../../migrations/0004_issues_and_pull_requests.sql"),
    ),
    (
        "0005_notifications_and_events",
        include_str!("../../migrations/0005_notifications_and_events.sql"),
    ),
    (
        "0006_artifacts_and_lfs",
        include_str!("../../migrations/0006_artifacts_and_lfs.sql"),
    ),
    (
        "0007_releases_and_packages",
        include_str!("../../migrations/0007_releases_and_packages.sql"),
    ),
    (
        "0008_workflows_and_runners",
        include_str!("../../migrations/0008_workflows_and_runners.sql"),
    ),
    (
        "0009_webhooks",
        include_str!("../../migrations/0009_webhooks.sql"),
    ),
    (
        "0010_discussions",
        include_str!("../../migrations/0010_discussions.sql"),
    ),
    (
        "0011_security_center",
        include_str!("../../migrations/0011_security_center.sql"),
    ),
];

// 64-bit advisory lock ID representing "SPINEMIG" (0x5350494E454D4947)
const SPINE_MIGRATION_ADVISORY_LOCK_ID: i64 = 0x5350_494E_454D_4947;

pub async fn run_migrations(pool: &PgPool) -> Result<(), ForgeError> {
    // Acquire a single dedicated connection so the session-level advisory lock stays held
    let mut conn = pool.acquire().await.map_err(|e| {
        ForgeError::Internal(format!("Failed to acquire connection for migrations: {}", e))
    })?;

    // Acquire session-level advisory lock to serialize concurrent API startup migrations
    tracing::info!("Acquiring cluster-wide schema migration advisory lock...");
    sqlx::query("SELECT pg_advisory_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .execute(&mut *conn)
        .await
        .map_err(|e| ForgeError::Internal(format!("Failed to acquire migration advisory lock: {}", e)))?;

    let res = run_migrations_inner(pool, &mut conn).await;

    // Release advisory lock
    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .execute(&mut *conn)
        .await;

    res
}

async fn run_migrations_inner(
    pool: &PgPool,
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
) -> Result<(), ForgeError> {
    // 1. Create _schema_migrations tracking table if not exists
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at_nanos BIGINT NOT NULL
        )
        "#,
    )
    .execute(&mut **conn)
    .await
    .map_err(|e| ForgeError::Internal(format!("Failed to create _schema_migrations table: {}", e)))?;

    // 2. Sequentially apply pending migrations within a transaction per migration
    for (name, sql) in MIGRATIONS {
        let exists: (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM _schema_migrations WHERE version = $1)",
        )
        .bind(name)
        .fetch_one(&mut **conn)
        .await
        .map_err(|e| ForgeError::Internal(format!("Failed to check migration status for {}: {}", name, e)))?;

        if !exists.0 {
            tracing::info!("Applying migration {}...", name);
            let mut tx = pool.begin().await.map_err(|e| {
                ForgeError::Internal(format!("Failed to begin migration transaction for {}: {}", name, e))
            })?;

            // Execute DDL batch
            sqlx::raw_sql(sql)
                .execute(&mut *tx)
                .await
                .map_err(|e| ForgeError::Internal(format!("Migration {} failed: {}", name, e)))?;

            let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
            sqlx::query("INSERT INTO _schema_migrations (version, applied_at_nanos) VALUES ($1, $2)")
                .bind(name)
                .bind(now)
                .execute(&mut *tx)
                .await
                .map_err(|e| ForgeError::Internal(format!("Failed to record migration {}: {}", name, e)))?;

            tx.commit().await.map_err(|e| {
                ForgeError::Internal(format!("Failed to commit migration transaction for {}: {}", name, e))
            })?;
            tracing::info!("Migration {} applied successfully.", name);
        }
    }

    Ok(())
}
