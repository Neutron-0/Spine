#![allow(dead_code)]

use sqlx::PgPool;
use url::Url;
use uuid::Uuid;

pub const HARNESS_PROOF_TABLE: &str = "_harness_provision_proof";
pub const DISPOSABLE_MARKER_TABLE: &str = "_spine_disposable_test_marker";

/// Authoritative, harness-owned disposable test database.
///
/// Guaranteed safety properties:
/// 1. The database was freshly created with a unique UUID (`spine_disp_<uuid>`) by the current test harness run.
/// 2. Destructive operations (DROP/TRUNCATE) refuse to operate on any database not created by this instance.
/// 3. Pre-existing databases (such as `spine_forge_test` or `spine_forge`) are rejected.
/// 4. When `cleanup().await` is invoked, only this uniquely provisioned database is removed.
pub struct HarnessDisposableDb {
    pub db_name: String,
    pub admin_url: String,
    pub migrator_url: String,
    pub app_url: String,
    pub pool: PgPool,
    pub app_pool: PgPool,
    pub creation_token: Uuid,
    created_by_current_harness: bool,
}

impl HarnessDisposableDb {
    /// Provisions an isolated, unique test database using a dedicated administrative connection.
    /// Applies migrations and configures least-privilege roles before returning.
    pub async fn provision() -> Result<Self, String> {
        Self::provision_internal(true).await
    }

    pub async fn provision_migrated() -> Result<Self, String> {
        Self::provision_internal(true).await
    }

    pub async fn provision_empty() -> Result<Self, String> {
        Self::provision_internal(false).await
    }

    pub async fn migrator_pool(&self) -> Result<PgPool, String> {
        Ok(self.pool.clone())
    }

    pub async fn app_pool(&self) -> Result<PgPool, String> {
        Ok(self.app_pool.clone())
    }

    async fn provision_internal(apply_migrations: bool) -> Result<Self, String> {
        let allow_destructive = std::env::var("SPINE_ALLOW_DESTRUCTIVE_TESTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !allow_destructive {
            return Err(
                "SAFETY ABORT: 'SPINE_ALLOW_DESTRUCTIVE_TESTS=1' must be explicitly set in the environment \
                 to provision a disposable test database."
                    .to_string(),
            );
        }

        let base_url = require_safe_test_db_url_internal()?;
        let mut parsed_admin =
            Url::parse(&base_url).map_err(|e| format!("Invalid base test database URL: {}", e))?;

        // Connect to maintenance database 'postgres' to provision the ephemeral database
        parsed_admin.set_path("postgres");
        let admin_url = parsed_admin.to_string();

        let admin_pool = PgPool::connect(&admin_url).await.map_err(|e| {
            format!(
                "Failed to connect to administrative database for provisioning: {}",
                e
            )
        })?;

        let creation_token = Uuid::new_v4();
        let db_name = format!("spine_disp_{}", &creation_token.simple().to_string()[..12]);

        // 1. Create unique database owned by spine_migrator
        let create_sql = format!(
            "CREATE DATABASE \"{}\" WITH OWNER = spine_migrator",
            db_name
        );
        sqlx::query(&create_sql)
            .execute(&admin_pool)
            .await
            .map_err(|e| {
                format!(
                    "Failed to create disposable test database '{}': {}",
                    db_name, e
                )
            })?;

        // 2. Build distinct connection URLs for the newly created database (no identity fallback)
        let mut parsed_migrator =
            Url::parse(&base_url).map_err(|e| format!("Invalid migrator URL: {}", e))?;
        parsed_migrator.set_path(&format!("/{}", db_name));
        let _ = parsed_migrator.set_username("spine_migrator");
        if let Ok(mig_pass) = std::env::var("SPINE_MIGRATOR_PASSWORD")
            .or_else(|_| std::env::var("SPINE_TEST_MIGRATOR_PASSWORD"))
        {
            let _ = parsed_migrator.set_password(Some(&mig_pass));
        }
        let migrator_url = parsed_migrator.to_string();

        let mut parsed_app =
            Url::parse(&base_url).map_err(|e| format!("Invalid app URL: {}", e))?;
        parsed_app.set_path(&format!("/{}", db_name));
        let _ = parsed_app.set_username("spine_app");
        if let Ok(app_pass) = std::env::var("SPINE_APP_PASSWORD")
            .or_else(|_| std::env::var("SPINE_TEST_APP_PASSWORD"))
        {
            let _ = parsed_app.set_password(Some(&app_pass));
        }
        let app_url = parsed_app.to_string();

        let pool = PgPool::connect(&migrator_url)
            .await
            .map_err(|e| format!("Failed to connect to newly created test database: {}", e))?;

        // Ensure spine_migrator has schema ownership and full DDL privileges on public
        let _ = sqlx::query("GRANT ALL ON SCHEMA public TO spine_migrator; ALTER SCHEMA public OWNER TO spine_migrator;")
            .execute(&pool)
            .await;

        // 3. Insert proof table proving this database was provisioned by this harness instance
        let proof_sql = format!(
            r#"
            CREATE TABLE IF NOT EXISTS "{}" (
                creation_token VARCHAR(64) PRIMARY KEY,
                process_pid INT NOT NULL,
                db_name VARCHAR(128) NOT NULL,
                created_at_nanos BIGINT NOT NULL
            )
            "#,
            HARNESS_PROOF_TABLE
        );
        sqlx::query(&proof_sql)
            .execute(&pool)
            .await
            .map_err(|e| format!("Failed to create harness proof table: {}", e))?;

        let insert_proof = format!(
            r#"
            INSERT INTO "{}" (creation_token, process_pid, db_name, created_at_nanos)
            VALUES ($1, $2, $3, $4)
            "#,
            HARNESS_PROOF_TABLE
        );
        let now_nanos = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0);
        sqlx::query(&insert_proof)
            .bind(creation_token.to_string())
            .bind(std::process::id() as i32)
            .bind(&db_name)
            .bind(now_nanos)
            .execute(&pool)
            .await
            .map_err(|e| format!("Failed to record harness provision proof: {}", e))?;

        // 4. Apply all baseline migrations if requested
        if apply_migrations {
            spine_api::db::migrations::run_migrations(&pool)
                .await
                .map_err(|e| {
                    format!(
                        "Failed to run migrations on newly created test database: {}",
                        e
                    )
                })?;
        }

        // 5. Grant least-privilege permissions to spine_app role
        let grant_sql = format!(
            r#"
            DO $$
            BEGIN
                IF EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_app') THEN
                    GRANT CONNECT ON DATABASE "{}" TO spine_app;
                    GRANT USAGE ON SCHEMA public TO spine_app;
                    REVOKE CREATE ON SCHEMA public FROM spine_app;
                    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO spine_app;
                    GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO spine_app;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO spine_app;
                    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO spine_app;
                END IF;
            END $$;
            "#,
            db_name
        );
        sqlx::query(&grant_sql)
            .execute(&pool)
            .await
            .map_err(|e| format!("Failed to configure spine_app permissions: {}", e))?;

        let app_pool = PgPool::connect(&app_url).await.map_err(|e| {
            format!(
                "Failed to connect to test database as spine_app ({}): {}",
                app_url, e
            )
        })?;

        Ok(Self {
            db_name,
            admin_url,
            migrator_url,
            app_url,
            pool,
            app_pool,
            creation_token,
            created_by_current_harness: true,
        })
    }

    /// Verifies that a target database was actively created by the current test harness invocation.
    /// Refuses to operate on pre-existing or foreign databases.
    pub async fn verify_can_destructively_modify(&self, check_pool: &PgPool) -> Result<(), String> {
        let allow_destructive = std::env::var("SPINE_ALLOW_DESTRUCTIVE_TESTS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if !allow_destructive {
            return Err(
                "SAFETY ABORT: Refusing destructive modification. 'SPINE_ALLOW_DESTRUCTIVE_TESTS=1' must be set."
                    .to_string(),
            );
        }

        if !self.created_by_current_harness {
            return Err(
                "SAFETY ABORT: Refusing destructive modification. Database was not created by this test harness."
                    .to_string(),
            );
        }

        let query = format!(
            "SELECT process_pid, db_name FROM \"{}\" WHERE creation_token = $1",
            HARNESS_PROOF_TABLE
        );
        let row: Result<Option<(i32, String)>, _> = sqlx::query_as(&query)
            .bind(self.creation_token.to_string())
            .fetch_optional(check_pool)
            .await;

        match row {
            Ok(Some((pid, name))) if pid == std::process::id() as i32 && name == self.db_name => {
                Ok(())
            }
            Ok(Some((pid, name))) => Err(format!(
                "SAFETY ABORT: Proof mismatch (pid {}, db '{}'). Expected (pid {}, db '{}').",
                pid,
                name,
                std::process::id(),
                self.db_name
            )),
            Ok(None) => Err(
                "SAFETY ABORT: No proof record found in database. Database is not disposable."
                    .to_string(),
            ),
            Err(e) => Err(format!(
                "SAFETY ABORT: Failed to verify harness proof: {}",
                e
            )),
        }
    }

    /// Explicitly verifies that a pre-existing database (such as `spine_forge_test`) is rejected.
    pub async fn verify_preexisting_rejected(preexisting_url: &str) -> Result<(), String> {
        let pool = match PgPool::connect(preexisting_url).await {
            Ok(p) => p,
            Err(e) => {
                return Err(format!(
                    "SAFETY ABORT: Pre-existing database '{}' rejected: Missing harness creation proof: {}",
                    preexisting_url, e
                ));
            }
        };

        // Check if it lacks the current process's harness proof
        let query = "SELECT EXISTS(SELECT 1 FROM information_schema.tables WHERE table_schema = 'public' AND table_name = $1)";
        let exists: (bool,) = sqlx::query_as(query)
            .bind(HARNESS_PROOF_TABLE)
            .fetch_one(&pool)
            .await
            .unwrap_or((false,));

        if !exists.0 {
            return Err(format!(
                "SAFETY ABORT: Pre-existing database '{}' rejected: Missing harness creation proof.",
                preexisting_url
            ));
        }

        Err(
            "SAFETY ABORT: Pre-existing database rejected: Missing harness creation proof."
                .to_string(),
        )
    }

    /// Cleanly drops the provisioned database using the administrative connection.
    pub async fn cleanup(self) -> Result<(), String> {
        self.pool.close().await;
        self.app_pool.close().await;

        let admin_pool = PgPool::connect(&self.admin_url)
            .await
            .map_err(|e| format!("Failed to connect to admin database for cleanup: {}", e))?;

        // Terminate any remaining backends connected to this database
        let terminate_sql =
            "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = $1 AND pid != pg_backend_pid()";
        let _ = sqlx::query(terminate_sql)
            .bind(&self.db_name)
            .execute(&admin_pool)
            .await;

        let drop_sql = format!("DROP DATABASE IF EXISTS \"{}\" WITH (FORCE)", self.db_name);
        sqlx::query(&drop_sql)
            .execute(&admin_pool)
            .await
            .map_err(|e| {
                format!(
                    "Failed to drop disposable test database '{}': {}",
                    self.db_name, e
                )
            })?;

        Ok(())
    }
}

/// Safely retrieve base test database URL from environment.
pub fn get_safe_test_db_url() -> Option<String> {
    require_safe_test_db_url_internal().ok()
}

pub fn require_safe_test_db_url() -> String {
    match require_safe_test_db_url_internal() {
        Ok(url) => url,
        Err(e) => panic!("{}", e),
    }
}

pub fn get_app_role_test_db_url() -> String {
    let mut parsed = Url::parse(&require_safe_test_db_url()).expect("valid test db url");
    let _ = parsed.set_username("spine_app");
    if let Ok(app_pass) =
        std::env::var("SPINE_APP_PASSWORD").or_else(|_| std::env::var("SPINE_TEST_APP_PASSWORD"))
    {
        let _ = parsed.set_password(Some(&app_pass));
    }
    parsed.to_string()
}

fn require_safe_test_db_url_internal() -> Result<String, String> {
    let raw_url = match std::env::var("SPINE_TEST_DATABASE_URL") {
        Ok(val) => val,
        Err(_) => {
            return Err(
                "FATAL TEST SAFETY ERROR: 'SPINE_TEST_DATABASE_URL' is not set.\n\
                 Database integration tests require an explicitly provisioned, isolated disposable database\n\
                 (e.g. postgres://spine_migrator:password@127.0.0.1:5432/spine_forge_test?sslmode=disable).\n\
                 Refusing to silently skip or pass without executing the real database path."
                    .to_string(),
            );
        }
    };

    let trimmed = raw_url.trim();
    if trimmed.is_empty() {
        return Err(
            "FATAL TEST SAFETY ERROR: 'SPINE_TEST_DATABASE_URL' is set but empty.".to_string(),
        );
    }

    let parsed =
        Url::parse(trimmed).map_err(|e| format!("Malformed SPINE_TEST_DATABASE_URL: {}", e))?;
    let db_name = parsed
        .path_segments()
        .and_then(|mut segs| segs.next())
        .unwrap_or("")
        .trim();

    if db_name.is_empty() {
        return Err(
            "SAFETY ABORT: SPINE_TEST_DATABASE_URL does not specify a database name.".to_string(),
        );
    }

    if db_name == "spine_forge" || db_name == "postgres" || db_name.starts_with("template") {
        return Err(format!(
            "SAFETY ABORT: Refusing to run tests against protected database '{}'.",
            db_name
        ));
    }

    Ok(trimmed.to_string())
}
