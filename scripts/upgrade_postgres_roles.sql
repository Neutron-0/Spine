-- ==============================================================================
-- Spine Forge - PostgreSQL Role Upgrade & Least-Privilege Hardening Script
-- Upgrades existing databases to separate DDL (migrator) and DML (app)
-- Supports configurable role names via -v migrator_user=... -v app_user=...
-- ==============================================================================

\if :{?migrator_user}
\else
  \set migrator_user spine_migrator
\endif

\if :{?app_user}
\else
  \set app_user spine_app
\endif

-- 1. Ensure dedicated migration role exists
DO $$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'migrator_user') THEN
        EXECUTE format('CREATE ROLE %I WITH LOGIN;', :'migrator_user');
    END IF;
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = :'app_user') THEN
        EXECUTE format('CREATE ROLE %I WITH LOGIN;', :'app_user');
    END IF;
END $$;

-- Enforce absence of administrative privileges
ALTER ROLE :"migrator_user" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
ALTER ROLE :"app_user" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- Transfer schema ownership to migrator role
ALTER SCHEMA public OWNER TO :"migrator_user";
GRANT ALL ON SCHEMA public TO :"migrator_user";

-- Reassign existing tables, sequences, and routines to migrator role
DO $$
DECLARE
    r RECORD;
BEGIN
    FOR r IN (SELECT tablename FROM pg_tables WHERE schemaname = 'public') LOOP
        EXECUTE format('ALTER TABLE public.%I OWNER TO %I;', r.tablename, :'migrator_user');
    END LOOP;

    FOR r IN (SELECT sequence_name FROM information_schema.sequences WHERE sequence_schema = 'public') LOOP
        EXECUTE format('ALTER SEQUENCE public.%I OWNER TO %I;', r.sequence_name, :'migrator_user');
    END LOOP;
END $$;

-- 3. Apply least-privilege grants to application role
GRANT USAGE ON SCHEMA public TO :"app_user";
REVOKE CREATE ON SCHEMA public FROM :"app_user";

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO :"app_user";
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO :"app_user";

ALTER DEFAULT PRIVILEGES FOR ROLE :"migrator_user" IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO :"app_user";

ALTER DEFAULT PRIVILEGES FOR ROLE :"migrator_user" IN SCHEMA public
    GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO :"app_user";
