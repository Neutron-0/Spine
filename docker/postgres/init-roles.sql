-- ==============================================================================
-- Spine Forge - Production PostgreSQL Role Initialization
-- Strictly enforces three distinct identities:
--   1. spine_admin    (Bootstrap administrator / superuser)
--   2. spine_migrator (DDL schema owner, migration runner)
--   3. spine_app      (Least-privilege runtime DML only)
-- ==============================================================================

-- 1. Create migration role with DDL privileges
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_migrator') THEN
        CREATE ROLE spine_migrator WITH LOGIN PASSWORD 'spine_migrator_secure_password';
    END IF;
END $$;

-- 2. Create application runtime role with DML privileges only
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_app') THEN
        CREATE ROLE spine_app WITH LOGIN PASSWORD 'spine_app_secure_password';
    END IF;
END $$;

-- Ensure roles cannot create databases or modify server config
ALTER ROLE spine_migrator NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
ALTER ROLE spine_app NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- Transfer database ownership to spine_migrator
ALTER DATABASE spine_forge OWNER TO spine_migrator;

\connect spine_forge

-- 3. Configure schema ownership and DDL rights
ALTER SCHEMA public OWNER TO spine_migrator;
GRANT ALL ON SCHEMA public TO spine_migrator;

-- 4. Apply strict least privilege to spine_app
GRANT CONNECT ON DATABASE spine_forge TO spine_app;
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
