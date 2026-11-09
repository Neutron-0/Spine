#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Spine Forge - Upgrade PostgreSQL Roles to Least Privilege
# ==============================================================================

if [ -n "${POSTGRES_DB:-}" ] && [ -n "${SPINE_POSTGRES_DB:-}" ] && [ "$POSTGRES_DB" != "$SPINE_POSTGRES_DB" ]; then
    echo "[ERROR] Conflicting database names: POSTGRES_DB='$POSTGRES_DB' but SPINE_POSTGRES_DB='$SPINE_POSTGRES_DB'" >&2
    exit 1
fi

DB_NAME="${POSTGRES_DB:-${SPINE_POSTGRES_DB:-spine_forge}}"
ADMIN_USER="${POSTGRES_ADMIN_USER:-postgres}"

echo "[INFO] Upgrading database '${DB_NAME}' roles using administrative user '${ADMIN_USER}'..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

psql -v ON_ERROR_STOP=1 -U "${ADMIN_USER}" -d "${DB_NAME}" -f "${SCRIPT_DIR}/upgrade_postgres_roles.sql"

# Update credentials securely via stdin if provided via environment or secret file
if [ -n "${POSTGRES_MIGRATOR_PASSWORD_FILE:-}" ]; then
    if [ ! -r "$POSTGRES_MIGRATOR_PASSWORD_FILE" ]; then
        echo "[ERROR] POSTGRES_MIGRATOR_PASSWORD_FILE is missing or unreadable: $POSTGRES_MIGRATOR_PASSWORD_FILE" >&2
        exit 1
    fi
    MIGRATOR_PASSWORD="$(cat "$POSTGRES_MIGRATOR_PASSWORD_FILE")"
    if [ -z "$MIGRATOR_PASSWORD" ]; then
        echo "[ERROR] POSTGRES_MIGRATOR_PASSWORD_FILE is empty" >&2
        exit 1
    fi
elif [ -n "${POSTGRES_MIGRATOR_PASSWORD:-}" ]; then
    MIGRATOR_PASSWORD="${POSTGRES_MIGRATOR_PASSWORD}"
else
    MIGRATOR_PASSWORD=""
fi

if [ -n "${POSTGRES_APP_PASSWORD_FILE:-}" ]; then
    if [ ! -r "$POSTGRES_APP_PASSWORD_FILE" ]; then
        echo "[ERROR] POSTGRES_APP_PASSWORD_FILE is missing or unreadable: $POSTGRES_APP_PASSWORD_FILE" >&2
        exit 1
    fi
    APP_PASSWORD="$(cat "$POSTGRES_APP_PASSWORD_FILE")"
    if [ -z "$APP_PASSWORD" ]; then
        echo "[ERROR] POSTGRES_APP_PASSWORD_FILE is empty" >&2
        exit 1
    fi
elif [ -n "${POSTGRES_APP_PASSWORD:-}" ]; then
    APP_PASSWORD="${POSTGRES_APP_PASSWORD}"
else
    APP_PASSWORD=""
fi

if [ -n "$MIGRATOR_PASSWORD" ]; then
    echo "[INFO] Updating spine_migrator password securely..."
    ESC_MIGRATOR_PASS="$(printf '%s' "$MIGRATOR_PASSWORD" | sed "s/'/''/g")"
    psql -v ON_ERROR_STOP=1 -U "${ADMIN_USER}" -d "${DB_NAME}" <<EOSQL
ALTER ROLE spine_migrator WITH PASSWORD '${ESC_MIGRATOR_PASS}';
EOSQL
fi

if [ -n "$APP_PASSWORD" ]; then
    echo "[INFO] Updating spine_app password securely..."
    ESC_APP_PASS="$(printf '%s' "$APP_PASSWORD" | sed "s/'/''/g")"
    psql -v ON_ERROR_STOP=1 -U "${ADMIN_USER}" -d "${DB_NAME}" <<EOSQL
ALTER ROLE spine_app WITH PASSWORD '${ESC_APP_PASS}';
EOSQL
fi

echo "[INFO] Verifying spine_app cannot create tables via non-destructive catalog query..."
CAN_CREATE="$(psql -U "${ADMIN_USER}" -d "${DB_NAME}" -t -A -c "SELECT has_schema_privilege('spine_app', 'public', 'CREATE');")"

if [ "$CAN_CREATE" = "t" ] || [ "$CAN_CREATE" = "true" ]; then
    echo "[ERROR] Verification failed: spine_app retains CREATE privilege on schema public!" >&2
    exit 1
else
    echo "[OK] Verified: spine_app is correctly forbidden from CREATE ON SCHEMA public (has_schema_privilege returned false)."
fi

echo "POSTGRES_ROLES_UPGRADE_SUCCESS"
