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
    (
        "0012_workflow_provenance",
        include_str!("../../migrations/0012_workflow_provenance.sql"),
    ),
];

// 64-bit advisory lock ID representing "SPINEMIG" (0x5350494E454D4947)
pub const SPINE_MIGRATION_ADVISORY_LOCK_ID: i64 = 0x5350_494E_454D_4947;

async fn acquire_lock(conn: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
        .execute(conn)
        .await?;
    Ok(())
}

async fn create_migrations_table(conn: &mut sqlx::PgConnection) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS _schema_migrations (
            version TEXT PRIMARY KEY,
            applied_at_nanos BIGINT NOT NULL
        )
        "#,
    )
    .execute(conn)
    .await?;
    Ok(())
}

async fn migration_exists(conn: &mut sqlx::PgConnection, name: &str) -> Result<bool, sqlx::Error> {
    let row: (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM _schema_migrations WHERE version = $1)")
            .bind(name)
            .fetch_one(conn)
            .await?;
    Ok(row.0)
}

async fn execute_migration(
    conn: &mut sqlx::PgConnection,
    name: &str,
    sql: &str,
) -> Result<(), sqlx::Error> {
    sqlx::raw_sql(sql).execute(&mut *conn).await?;
    let now = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
    sqlx::query("INSERT INTO _schema_migrations (version, applied_at_nanos) VALUES ($1, $2)")
        .bind(name)
        .bind(now)
        .execute(conn)
        .await?;
    Ok(())
}

pub async fn run_migrations(pool: &PgPool) -> Result<(), ForgeError> {
    run_migrations_with_pause(pool.clone(), None, None).await
}

pub async fn run_migrations_with_pause(
    pool: PgPool,
    pause_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    resume_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), ForgeError> {
    run_migrations_with_pause_internal(pool, None, pause_notify, resume_notify).await
}

pub async fn run_migrations_with_pause_after_step(
    pool: PgPool,
    pause_after_step: usize,
    pause_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    resume_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), ForgeError> {
    run_migrations_with_pause_internal(pool, Some(pause_after_step), pause_notify, resume_notify)
        .await
}

async fn run_migrations_with_pause_internal(
    pool: PgPool,
    pause_after_step: Option<usize>,
    pause_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
    resume_notify: Option<std::sync::Arc<tokio::sync::Notify>>,
) -> Result<(), ForgeError> {
    let mut tx = pool.begin().await.map_err(|e| {
        ForgeError::Internal(format!("Failed to begin transaction for migrations: {}", e))
    })?;

    // Acquire transaction-scoped advisory lock (pg_advisory_xact_lock) to serialize concurrent API
    // startup migrations. This is automatically and instantly released when the transaction commits,
    // aborts, or is dropped upon future cancellation.
    tracing::info!("Acquiring cluster-wide schema migration transaction advisory lock...");
    acquire_lock(&mut tx).await.map_err(|e| {
        ForgeError::Internal(format!(
            "Failed to acquire migration transaction advisory lock: {}",
            e
        ))
    })?;

    // If pause is requested before any DDL
    if pause_after_step.is_none() {
        if let Some(ref pause) = pause_notify {
            pause.notify_one();
        }
        if let Some(ref resume) = resume_notify {
            resume.notified().await;
        }
    }

    // 1. Create _schema_migrations tracking table if not exists
    create_migrations_table(&mut tx).await.map_err(|e| {
        ForgeError::Internal(format!("Failed to create _schema_migrations table: {}", e))
    })?;

    // 2. Sequentially apply pending migrations within this transaction
    let mut step_count = 0usize;
    for (name, sql) in MIGRATIONS {
        let exists = migration_exists(&mut tx, name).await.map_err(|e| {
            ForgeError::Internal(format!(
                "Failed to check migration status for {}: {}",
                name, e
            ))
        })?;

        if !exists {
            tracing::info!("Applying migration {}...", name);
            execute_migration(&mut tx, name, sql)
                .await
                .map_err(|e| ForgeError::Internal(format!("Migration {} failed: {}", name, e)))?;
            tracing::info!("Migration {} applied successfully.", name);

            if let Some(target_step) = pause_after_step {
                if step_count == target_step {
                    if let Some(ref pause) = pause_notify {
                        pause.notify_one();
                    }
                    if let Some(ref resume) = resume_notify {
                        resume.notified().await;
                    }
                }
            }
            step_count += 1;
        }
    }

    tx.commit().await.map_err(|e| {
        ForgeError::Internal(format!("Failed to commit migration transaction: {}", e))
    })?;

    tracing::info!("All schema migrations applied and transaction committed.");
    Ok(())
}
