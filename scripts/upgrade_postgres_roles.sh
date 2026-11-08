#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Spine Forge - Upgrade PostgreSQL Roles to Least Privilege
# ==============================================================================

DB_NAME="${SPINE_POSTGRES_DB:-spine_forge}"
ADMIN_USER="${POSTGRES_ADMIN_USER:-postgres}"

echo "[INFO] Upgrading database '${DB_NAME}' roles using administrative user '${ADMIN_USER}'..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

psql -v ON_ERROR_STOP=1 -U "${ADMIN_USER}" -d "${DB_NAME}" -f "${SCRIPT_DIR}/upgrade_postgres_roles.sql"

echo "[INFO] Verifying spine_app cannot create tables via non-destructive catalog query..."
CAN_CREATE="$(psql -U "${ADMIN_USER}" -d "${DB_NAME}" -t -A -c "SELECT has_schema_privilege('spine_app', 'public', 'CREATE');")"

if [ "$CAN_CREATE" = "t" ] || [ "$CAN_CREATE" = "true" ]; then
    echo "[ERROR] Verification failed: spine_app retains CREATE privilege on schema public!" >&2
    exit 1
else
    echo "[OK] Verified: spine_app is correctly forbidden from CREATE ON SCHEMA public (has_schema_privilege returned false)."
fi

echo "POSTGRES_ROLES_UPGRADE_SUCCESS"
