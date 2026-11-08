-- ==============================================================================
-- Spine Forge - PostgreSQL Role Upgrade & Least-Privilege Hardening Script
-- Upgrades existing databases to separate DDL (spine_migrator) and DML (spine_app)
-- ==============================================================================

-- 1. Ensure dedicated migration role exists
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_migrator') THEN
        CREATE ROLE spine_migrator WITH LOGIN;
    END IF;
END $$;

-- 2. Ensure application role exists
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'spine_app') THEN
        CREATE ROLE spine_app WITH LOGIN;
    END IF;
END $$;

-- Enforce absence of administrative privileges
ALTER ROLE spine_migrator NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
ALTER ROLE spine_app NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- Transfer schema ownership to spine_migrator
ALTER SCHEMA public OWNER TO spine_migrator;
GRANT ALL ON SCHEMA public TO spine_migrator;

-- Reassign existing tables, sequences, and routines to spine_migrator
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public') LOOP
        EXECUTE format('ALTER TABLE public.%I OWNER TO spine_migrator;', r.tablename);
    END LOOP;

    FOR r IN (SELECT sequence_name FROM information_schema.sequences WHERE sequence_schema = 'public') LOOP
        EXECUTE format('ALTER SEQUENCE public.%I OWNER TO spine_migrator;', r.sequence_name);
    END LOOP;
END $$;

-- 3. Apply least-privilege grants to spine_app
GRANT USAGE ON SCHEMA public TO spine_app;
REVOKE CREATE ON SCHEMA public FROM spine_app;

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO spine_app;
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO spine_app;

ALTER DEFAULT PRIVILEGES FOR ROLE spine_migrator IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO spine_app;

ALTER DEFAULT PRIVILEGES FOR ROLE spine_migrator IN SCHEMA public
    GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO spine_app;
