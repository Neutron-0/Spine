-- ==============================================================================
-- Spine Forge - Production PostgreSQL Role Initialization
-- Strictly enforces three distinct identities:
--   1. spine_admin    (Bootstrap administrator / superuser)
--   2. spine_migrator (DDL schema owner, migration runner)
--   3. spine_app      (Least-privilege runtime DML only)
--
-- Password credentials and database names are loaded dynamically from environment
-- or psql variables (e.g. via 01-init-roles.sh or POSTGRES_* environment variables).
-- No hardcoded credentials permitted.
-- ==============================================================================

\getenv db_name POSTGRES_DB
\if :{?db_name}
\else
\set db_name spine_forge
\endif

\getenv migrator_pass POSTGRES_MIGRATOR_PASSWORD
\if :{?migrator_pass}
\else
\getenv migrator_pass POSTGRES_PASSWORD
\endif

\getenv app_pass POSTGRES_APP_PASSWORD
\if :{?app_pass}
\else
\getenv app_pass POSTGRES_PASSWORD
\endif

-- 1. Create migration role with DDL privileges
DO $$
DECLARE
    m_pass text := :'migrator_pass';
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_migrator') THEN
        EXECUTE format('CREATE ROLE spine_migrator WITH LOGIN PASSWORD %L', m_pass);
    ELSE
        EXECUTE format('ALTER ROLE spine_migrator WITH PASSWORD %L', m_pass);
    END IF;
END $$;

-- 2. Create application runtime role with DML privileges only
DO $$
DECLARE
    a_pass text := :'app_pass';
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_app') THEN
        EXECUTE format('CREATE ROLE spine_app WITH LOGIN PASSWORD %L', a_pass);
    ELSE
        EXECUTE format('ALTER ROLE spine_app WITH PASSWORD %L', a_pass);
    END IF;
END $$;

-- Ensure roles cannot create databases or modify server config
ALTER ROLE spine_migrator NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
ALTER ROLE spine_app NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- Transfer database ownership to spine_migrator
EXECUTE format('ALTER DATABASE %I OWNER TO spine_migrator', :'db_name');

\connect :db_name

-- 3. Configure schema ownership and DDL rights
ALTER SCHEMA public OWNER TO spine_migrator;
GRANT ALL ON SCHEMA public TO spine_migrator;

-- 4. Apply strict least privilege to spine_app
EXECUTE format('GRANT CONNECT ON DATABASE %I TO spine_app', :'db_name');
GRANT USAGE ON SCHEMA public TO spine_app;
REVOKE CREATE ON SCHEMA public FROM spine_app;

-- Ensure spine_app has DML privileges on all existing objects
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO spine_app;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO spine_app;

-- Configure default privileges so future tables and sequences created by spine_migrator are accessible to spine_app
ALTER DEFAULT PRIVILEGES FOR ROLE spine_migrator IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO spine_app;

ALTER DEFAULT PRIVILEGES FOR ROLE spine_migrator IN SCHEMA public
    GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO spine_app;
