#!/usr/bin/env sh
# ==============================================================================
# Spine Forge - Parameterized PostgreSQL Role Initialization
# Creates distinct least-privilege identities dynamically from environment / secrets:
#   1. spine_admin    (Bootstrap administrator / superuser from POSTGRES_USER)
#   2. spine_migrator (DDL schema owner, migration runner)
#   3. spine_app      (Least-privilege runtime DML only)
# ==============================================================================
set -e

DB_NAME="${POSTGRES_DB:-spine_forge}"
ADMIN_USER="${POSTGRES_USER:-spine_admin}"
MIGRATOR_USER="${POSTGRES_MIGRATOR_USER:-spine_migrator}"
APP_USER="${POSTGRES_APP_USER:-spine_app}"

# Support reading credentials from secret files or environment variables without falling back to admin credentials
if [ -n "${POSTGRES_MIGRATOR_PASSWORD_FILE:-}" ]; then
    if [ ! -r "$POSTGRES_MIGRATOR_PASSWORD_FILE" ]; then
        echo "FATAL: Configured POSTGRES_MIGRATOR_PASSWORD_FILE is missing or unreadable: $POSTGRES_MIGRATOR_PASSWORD_FILE" >&2
        exit 1
    fi
    MIGRATOR_PASSWORD="$(cat "$POSTGRES_MIGRATOR_PASSWORD_FILE")"
elif [ -n "${POSTGRES_MIGRATOR_PASSWORD:-}" ]; then
    MIGRATOR_PASSWORD="${POSTGRES_MIGRATOR_PASSWORD}"
elif [ -n "${SPINE_MIGRATOR_PASSWORD:-}" ]; then
    MIGRATOR_PASSWORD="${SPINE_MIGRATOR_PASSWORD}"
else
    echo "FATAL: POSTGRES_MIGRATOR_PASSWORD or POSTGRES_MIGRATOR_PASSWORD_FILE is required (no fallback to admin password permitted)" >&2
    exit 1
fi

if [ -n "${POSTGRES_APP_PASSWORD_FILE:-}" ]; then
    if [ ! -r "$POSTGRES_APP_PASSWORD_FILE" ]; then
        echo "FATAL: Configured POSTGRES_APP_PASSWORD_FILE is missing or unreadable: $POSTGRES_APP_PASSWORD_FILE" >&2
        exit 1
    fi
    APP_PASSWORD="$(cat "$POSTGRES_APP_PASSWORD_FILE")"
elif [ -n "${POSTGRES_APP_PASSWORD:-}" ]; then
    APP_PASSWORD="${POSTGRES_APP_PASSWORD}"
elif [ -n "${SPINE_APP_PASSWORD:-}" ]; then
    APP_PASSWORD="${SPINE_APP_PASSWORD}"
else
    echo "FATAL: POSTGRES_APP_PASSWORD or POSTGRES_APP_PASSWORD_FILE is required (no fallback to admin password permitted)" >&2
    exit 1
fi

echo "Initializing dynamic PostgreSQL roles for database '${DB_NAME}' with safe SQL identifier/literal quoting via \\gexec..."

# 1. Create or update migrator and app roles using safe PostgreSQL format() and \gexec
# Variables are passed to format() outside dollar-quoted blocks to prevent psql expansion failures.
psql -v ON_ERROR_STOP=1 \
     -v db_name="$DB_NAME" \
     -v migrator_user="$MIGRATOR_USER" \
     -v migrator_password="$MIGRATOR_PASSWORD" \
     -v app_user="$APP_USER" \
     -v app_password="$APP_PASSWORD" \
     --username "$ADMIN_USER" --dbname "$DB_NAME" <<-'EOSQL'
    -- Create or alter migrator role
    SELECT format(
        'DO $B$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = %L) THEN
                CREATE ROLE %I WITH LOGIN PASSWORD %L;
            ELSE
                ALTER ROLE %I WITH PASSWORD %L;
            END IF;
        END $B$;',
        :'migrator_user', :'migrator_user', :'migrator_password',
        :'migrator_user', :'migrator_password'
    ) \gexec

    -- Create or alter application runtime role
    SELECT format(
        'DO $B$ BEGIN
            IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = %L) THEN
                CREATE ROLE %I WITH LOGIN PASSWORD %L;
            ELSE
                ALTER ROLE %I WITH PASSWORD %L;
            END IF;
        END $B$;',
        :'app_user', :'app_user', :'app_password',
        :'app_user', :'app_password'
    ) \gexec

    -- Enforce non-admin privileges
    SELECT format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;', :'migrator_user') \gexec
    SELECT format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;', :'app_user') \gexec

    -- Transfer database ownership to migrator role
    SELECT format('ALTER DATABASE %I OWNER TO %I;', :'db_name', :'migrator_user') \gexec

    -- Schema ownership and DDL rights
    SELECT format('ALTER SCHEMA public OWNER TO %I;', :'migrator_user') \gexec
    SELECT format('GRANT ALL ON SCHEMA public TO %I;', :'migrator_user') \gexec

    -- Strict least privilege for application role
    SELECT format('GRANT CONNECT ON DATABASE %I TO %I;', :'db_name', :'app_user') \gexec
    SELECT format('GRANT USAGE ON SCHEMA public TO %I;', :'app_user') \gexec
    SELECT format('REVOKE CREATE ON SCHEMA public FROM %I;', :'app_user') \gexec

    -- DML permissions on existing tables and sequences
    SELECT format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO %I;', :'app_user') \gexec
    SELECT format('GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO %I;', :'app_user') \gexec

    -- Default privileges for future objects created by migrator
    SELECT format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I;', :'migrator_user', :'app_user') \gexec
    SELECT format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO %I;', :'migrator_user', :'app_user') \gexec
EOSQL

echo "Role provisioning complete for database '${DB_NAME}'."
