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

# Support reading credentials from secret files or environment variables
if [ -n "${POSTGRES_MIGRATOR_PASSWORD_FILE:-}" ] && [ -f "$POSTGRES_MIGRATOR_PASSWORD_FILE" ]; then
    MIGRATOR_PASSWORD="$(cat "$POSTGRES_MIGRATOR_PASSWORD_FILE")"
else
    MIGRATOR_PASSWORD="${POSTGRES_MIGRATOR_PASSWORD:-${POSTGRES_PASSWORD}}"
fi

if [ -n "${POSTGRES_APP_PASSWORD_FILE:-}" ] && [ -f "$POSTGRES_APP_PASSWORD_FILE" ]; then
    APP_PASSWORD="$(cat "$POSTGRES_APP_PASSWORD_FILE")"
else
    APP_PASSWORD="${POSTGRES_APP_PASSWORD:-${POSTGRES_PASSWORD}}"
fi

echo "Initializing dynamic PostgreSQL roles for database '${DB_NAME}'..."

psql -v ON_ERROR_STOP=1 --username "$ADMIN_USER" --dbname "$DB_NAME" <<-EOSQL
    -- 1. Create migration role with DDL privileges
    DO \$\$
    BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '${MIGRATOR_USER}') THEN
            CREATE ROLE "${MIGRATOR_USER}" WITH LOGIN PASSWORD '${MIGRATOR_PASSWORD}';
        ELSE
            ALTER ROLE "${MIGRATOR_USER}" WITH PASSWORD '${MIGRATOR_PASSWORD}';
        END IF;
    END \$\$;

    -- 2. Create application runtime role with DML privileges only
    DO \$\$
    BEGIN
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '${APP_USER}') THEN
            CREATE ROLE "${APP_USER}" WITH LOGIN PASSWORD '${APP_PASSWORD}';
        ELSE
            ALTER ROLE "${APP_USER}" WITH PASSWORD '${APP_PASSWORD}';
        END IF;
    END \$\$;

    -- Ensure non-admin roles cannot create databases or modify server config
    ALTER ROLE "${MIGRATOR_USER}" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
    ALTER ROLE "${APP_USER}" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

    -- Transfer database ownership to migrator role
    ALTER DATABASE "${DB_NAME}" OWNER TO "${MIGRATOR_USER}";
EOSQL

psql -v ON_ERROR_STOP=1 --username "$ADMIN_USER" --dbname "${DB_NAME}" <<-EOSQL
    -- 3. Configure schema ownership and DDL rights
    ALTER SCHEMA public OWNER TO "${MIGRATOR_USER}";
    GRANT ALL ON SCHEMA public TO "${MIGRATOR_USER}";

    -- 4. Apply strict least privilege to app role
    GRANT CONNECT ON DATABASE "${DB_NAME}" TO "${APP_USER}";
    GRANT USAGE ON SCHEMA public TO "${APP_USER}";
    REVOKE CREATE ON SCHEMA public FROM "${APP_USER}";

    -- Ensure app role has DML privileges on all existing objects
    GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO "${APP_USER}";
    GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO "${APP_USER}";

    -- Configure default privileges for future objects
    ALTER DEFAULT PRIVILEGES FOR ROLE "${MIGRATOR_USER}" IN SCHEMA public
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO "${APP_USER}";

    ALTER DEFAULT PRIVILEGES FOR ROLE "${MIGRATOR_USER}" IN SCHEMA public
        GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO "${APP_USER}";
EOSQL

echo "Role provisioning complete for database '${DB_NAME}'."
