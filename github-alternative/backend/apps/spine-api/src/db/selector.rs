use crate::core::error::ForgeError;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::path::{Path, PathBuf};
use url::Url;

pub const SELECTOR_FILE_NAME: &str = "active_database_selector.json";
pub const CONTROL_DIR_NAME: &str = ".spine_restore_control";
pub const SELECTOR_TABLE_NAME: &str = "_spine_active_database_selector";

/// Authoritative record identifying the active PostgreSQL database.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ActiveDatabaseRecord {
    pub base_db_name: String,
    pub active_db_name: String,
    pub active_generation: String,
    pub restore_id: String,
    pub cutover_at_nanos: i64,
}

pub struct DatabaseSelector;

impl DatabaseSelector {
    /// Return the control file path in the storage root.
    pub fn selector_file_path(storage_root: &Path) -> PathBuf {
        storage_root.join(CONTROL_DIR_NAME).join(SELECTOR_FILE_NAME)
    }

    /// Read the active database record from disk.
    pub fn load_from_file(storage_root: &Path) -> Result<Option<ActiveDatabaseRecord>, ForgeError> {
        let path = Self::selector_file_path(storage_root);
        if !path.exists() {
            return Ok(None);
        }
        let content = std::fs::read_to_string(&path).map_err(|e| {
            ForgeError::Internal(format!("Failed to read active database selector: {}", e))
        })?;
        let record = serde_json::from_str::<ActiveDatabaseRecord>(&content).map_err(|e| {
            ForgeError::Internal(format!("Corrupt active database selector: {}", e))
        })?;
        Ok(Some(record))
    }

    /// Persist the active database record to file using atomic replacement and sync.
    pub fn save_to_file(
        storage_root: &Path,
        record: &ActiveDatabaseRecord,
    ) -> Result<(), ForgeError> {
        let path = Self::selector_file_path(storage_root);
        let parent = path.parent().unwrap_or_else(|| Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| ForgeError::Internal(e.to_string()))?;

        let json = serde_json::to_string_pretty(record).map_err(|e| {
            ForgeError::Internal(format!("Failed to serialize database selector: {}", e))
        })?;
        let tmp = parent.join(format!(".tmp_sel_{}.json", uuid::Uuid::new_v4().simple()));
        {
            use std::io::Write;
            let mut f = std::fs::File::create(&tmp).map_err(|e| {
                ForgeError::Internal(format!("Failed to create temporary selector: {}", e))
            })?;
            f.write_all(json.as_bytes())
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
            f.flush().map_err(|e| ForgeError::Internal(e.to_string()))?;
            f.sync_all()
                .map_err(|e| ForgeError::Internal(e.to_string()))?;
        }
        crate::backup::safe_atomic_replace(&tmp, &path)?;
        Ok(())
    }

    /// Query the dedicated control database table in PostgreSQL if available.
    pub async fn load_from_db(
        maint_pool: &PgPool,
    ) -> Result<Option<ActiveDatabaseRecord>, ForgeError> {
        let table_exists: Option<(bool,)> = sqlx::query_as(
            "SELECT EXISTS (SELECT FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)",
        )
        .bind(SELECTOR_TABLE_NAME)
        .fetch_optional(maint_pool)
        .await
        .ok()
        .flatten();

        if table_exists.map(|t| t.0).unwrap_or(false) {
            let row: Option<(String, String, String, String, i64)> = sqlx::query_as(
                "SELECT base_db_name, active_db_name, active_generation, restore_id, cutover_at_nanos FROM _spine_active_database_selector WHERE id = 1",
            )
            .fetch_optional(maint_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to query _spine_active_database_selector: {}",
                    e
                ))
            })?;

            if let Some((base, active, gen, rst, nanos)) = row {
                return Ok(Some(ActiveDatabaseRecord {
                    base_db_name: base,
                    active_db_name: active,
                    active_generation: gen,
                    restore_id: rst,
                    cutover_at_nanos: nanos,
                }));
            }
        }
        Ok(None)
    }

    /// Persist the active database record to control database table in a single atomic transaction.
    pub async fn save_to_db(
        maint_pool: &PgPool,
        record: &ActiveDatabaseRecord,
    ) -> Result<(), ForgeError> {
        let ddl = format!(
            r#"
            CREATE TABLE IF NOT EXISTS "{}" (
                id INT PRIMARY KEY DEFAULT 1,
                base_db_name TEXT NOT NULL,
                active_db_name TEXT NOT NULL,
                active_generation TEXT NOT NULL,
                restore_id TEXT NOT NULL,
                cutover_at_nanos BIGINT NOT NULL
            );
            "#,
            SELECTOR_TABLE_NAME
        );
        sqlx::query(&ddl)
            .execute(maint_pool)
            .await
            .map_err(|e| ForgeError::Internal(format!("Failed to create selector table: {}", e)))?;

        let upsert = format!(
            r#"
            INSERT INTO "{}" (id, base_db_name, active_db_name, active_generation, restore_id, cutover_at_nanos)
            VALUES (1, $1, $2, $3, $4, $5)
            ON CONFLICT (id) DO UPDATE SET
                base_db_name = EXCLUDED.base_db_name,
                active_db_name = EXCLUDED.active_db_name,
                active_generation = EXCLUDED.active_generation,
                restore_id = EXCLUDED.restore_id,
                cutover_at_nanos = EXCLUDED.cutover_at_nanos;
            "#,
            SELECTOR_TABLE_NAME
        );
        sqlx::query(&upsert)
            .bind(&record.base_db_name)
            .bind(&record.active_db_name)
            .bind(&record.active_generation)
            .bind(&record.restore_id)
            .bind(record.cutover_at_nanos)
            .execute(maint_pool)
            .await
            .map_err(|e| {
                ForgeError::Internal(format!(
                    "Failed to record active database in control table: {}",
                    e
                ))
            })?;

        Ok(())
    }

    /// Resolve the authoritative active database connection URL.
    /// Inspects both control database and storage root selector file.
    pub async fn resolve_active_db_url(
        configured_url: &str,
        storage_root: &Path,
    ) -> Result<String, ForgeError> {
        let mut parsed = Url::parse(configured_url)
            .map_err(|e| ForgeError::Internal(format!("Invalid PostgreSQL URL: {}", e)))?;
        let base_db_name = parsed.path().trim_start_matches('/').to_string();
        if base_db_name.is_empty() {
            return Err(ForgeError::Internal(
                "PostgreSQL URL missing database name".into(),
            ));
        }

        // 1. Try loading from control database
        let mut maint_url = parsed.clone();
        maint_url.set_path("postgres");
        if let Ok(maint_pool) = PgPool::connect(maint_url.as_str()).await {
            if let Ok(Some(rec)) = Self::load_from_db(&maint_pool).await {
                if rec.base_db_name == base_db_name {
                    parsed.set_path(&rec.active_db_name);
                    maint_pool.close().await;
                    return Ok(parsed.to_string());
                }
            }
            maint_pool.close().await;
        }

        // 2. Try loading from storage root control file
        if let Ok(Some(rec)) = Self::load_from_file(storage_root) {
            if rec.base_db_name == base_db_name {
                parsed.set_path(&rec.active_db_name);
                return Ok(parsed.to_string());
            }
        }

        // 3. Fallback to configured base database name
        Ok(configured_url.to_string())
    }
}
