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

if [ -z "$MIGRATOR_PASSWORD" ]; then
    echo "FATAL: Resolved MIGRATOR_PASSWORD is empty" >&2
    exit 1
fi
if [ -z "$APP_PASSWORD" ]; then
    echo "FATAL: Resolved APP_PASSWORD is empty" >&2
    exit 1
fi

echo "Initializing dynamic PostgreSQL roles for database '${DB_NAME}' without command-line password exposure..."

# Helper function to escape single quotes in SQL string literals
escape_sql_literal() {
    printf '%s' "$1" | sed "s/'/''/g"
}

# Helper function to escape double quotes in SQL identifiers
escape_sql_identifier() {
    printf '%s' "$1" | sed 's/"/""/g'
}

ESC_MIGRATOR_PASS="$(escape_sql_literal "$MIGRATOR_PASSWORD")"
ESC_APP_PASS="$(escape_sql_literal "$APP_PASSWORD")"
ESC_MIGRATOR_USER="$(escape_sql_identifier "$MIGRATOR_USER")"
ESC_APP_USER="$(escape_sql_identifier "$APP_USER")"
ESC_DB_NAME="$(escape_sql_identifier "$DB_NAME")"

# Execute role initialization by streaming SQL over stdin.
# Passwords are NEVER passed via command line flags (-v) to prevent exposure in /proc/cmdline and ps.
psql -v ON_ERROR_STOP=1 --username "$ADMIN_USER" --dbname "$DB_NAME" <<EOSQL
DO \$B\$
BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '${ESC_MIGRATOR_USER}') THEN
        CREATE ROLE "${ESC_MIGRATOR_USER}" WITH LOGIN PASSWORD '${ESC_MIGRATOR_PASS}';
    ELSE
        ALTER ROLE "${ESC_MIGRATOR_USER}" WITH PASSWORD '${ESC_MIGRATOR_PASS}';
    END IF;

    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '${ESC_APP_USER}') THEN
        CREATE ROLE "${ESC_APP_USER}" WITH LOGIN PASSWORD '${ESC_APP_PASS}';
    ELSE
        ALTER ROLE "${ESC_APP_USER}" WITH PASSWORD '${ESC_APP_PASS}';
    END IF;
END \$B\$;

ALTER ROLE "${ESC_MIGRATOR_USER}" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;
ALTER ROLE "${ESC_APP_USER}" NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

ALTER DATABASE "${ESC_DB_NAME}" OWNER TO "${ESC_MIGRATOR_USER}";

ALTER SCHEMA public OWNER TO "${ESC_MIGRATOR_USER}";
GRANT ALL ON SCHEMA public TO "${ESC_MIGRATOR_USER}";

GRANT CONNECT ON DATABASE "${ESC_DB_NAME}" TO "${ESC_APP_USER}";
GRANT USAGE ON SCHEMA public TO "${ESC_APP_USER}";
REVOKE CREATE ON SCHEMA public FROM "${ESC_APP_USER}";

GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO "${ESC_APP_USER}";
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO "${ESC_APP_USER}";

ALTER DEFAULT PRIVILEGES FOR ROLE "${ESC_MIGRATOR_USER}" IN SCHEMA public
    GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO "${ESC_APP_USER}";
ALTER DEFAULT PRIVILEGES FOR ROLE "${ESC_MIGRATOR_USER}" IN SCHEMA public
    GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO "${ESC_APP_USER}";
EOSQL

echo "Role provisioning complete for database '${DB_NAME}'."
