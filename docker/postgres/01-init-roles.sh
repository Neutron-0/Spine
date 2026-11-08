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

echo "Initializing dynamic PostgreSQL roles for database '${DB_NAME}' with safe SQL identifier/literal quoting..."

psql -v ON_ERROR_STOP=1 \
     -v db_name="$DB_NAME" \
     -v migrator_user="$MIGRATOR_USER" \
     -v migrator_password="$MIGRATOR_PASSWORD" \
     -v app_user="$APP_USER" \
     -v app_password="$APP_PASSWORD" \
     --username "$ADMIN_USER" --dbname "$DB_NAME" <<-EOSQL
    -- 1. Create or update migrator and app roles using safe PostgreSQL format() engine
    DO \$ROLE_BLOCK\$
    DECLARE
        v_db text := :'db_name';
        v_migrator text := :'migrator_user';
        v_migrator_pass text := :'migrator_password';
        v_app text := :'app_user';
        v_app_pass text := :'app_password';
    BEGIN
        -- Migrator identity
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = v_migrator) THEN
            EXECUTE format('CREATE ROLE %I WITH LOGIN PASSWORD %L', v_migrator, v_migrator_pass);
        ELSE
            EXECUTE format('ALTER ROLE %I WITH PASSWORD %L', v_migrator, v_migrator_pass);
        END IF;

        -- Application runtime identity
        IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = v_app) THEN
            EXECUTE format('CREATE ROLE %I WITH LOGIN PASSWORD %L', v_app, v_app_pass);
        ELSE
            EXECUTE format('ALTER ROLE %I WITH PASSWORD %L', v_app, v_app_pass);
        END IF;

        -- Ensure non-admin roles cannot create databases or modify server config
        EXECUTE format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS', v_migrator);
        EXECUTE format('ALTER ROLE %I NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS', v_app);

        -- Transfer database ownership to migrator role
        EXECUTE format('ALTER DATABASE %I OWNER TO %I', v_db, v_migrator);
    END \$ROLE_BLOCK\$;
EOSQL

psql -v ON_ERROR_STOP=1 \
     -v db_name="$DB_NAME" \
     -v migrator_user="$MIGRATOR_USER" \
     -v app_user="$APP_USER" \
     --username "$ADMIN_USER" --dbname "${DB_NAME}" <<-EOSQL
    -- 2. Configure schema ownership, grants, and least-privilege defaults safely
    DO \$PERM_BLOCK\$
    DECLARE
        v_db text := :'db_name';
        v_migrator text := :'migrator_user';
        v_app text := :'app_user';
    BEGIN
        -- Schema ownership and DDL rights
        EXECUTE format('ALTER SCHEMA public OWNER TO %I', v_migrator);
        EXECUTE format('GRANT ALL ON SCHEMA public TO %I', v_migrator);

        -- Strict least privilege for application role
        EXECUTE format('GRANT CONNECT ON DATABASE %I TO %I', v_db, v_app);
        EXECUTE format('GRANT USAGE ON SCHEMA public TO %I', v_app);
        EXECUTE format('REVOKE CREATE ON SCHEMA public FROM %I', v_app);

        -- DML permissions on existing tables and sequences
        EXECUTE format('GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO %I', v_app);
        EXECUTE format('GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO %I', v_app);

        -- Configure default privileges for future objects created by migrator
        EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO %I', v_migrator, v_app);
        EXECUTE format('ALTER DEFAULT PRIVILEGES FOR ROLE %I IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO %I', v_migrator, v_app);
    END \$PERM_BLOCK\$;
EOSQL

echo "Role provisioning complete for database '${DB_NAME}'."
