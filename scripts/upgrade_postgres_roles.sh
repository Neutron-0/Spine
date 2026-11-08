#!/usr/bin/env bash
set -euo pipefail

# ==============================================================================
# Spine Forge - Upgrade PostgreSQL Roles to Least Privilege
# ==============================================================================

DB_NAME="${SPINE_POSTGRES_DB:-spine_forge}"
ADMIN_USER="${POSTGRES_ADMIN_USER:-postgres}"

echo "[INFO] Upgrading database '${DB_NAME}' roles using administrative user '${ADMIN_USER}'..."
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

psql -U "${ADMIN_USER}" -d "${DB_NAME}" -f "${SCRIPT_DIR}/upgrade_postgres_roles.sql"

echo "[INFO] Verifying spine_app cannot create tables..."
if psql -U spine_app -d "${DB_NAME}" -c "CREATE TABLE _unauthorized_test (id int);" 2>/dev/null; then
    echo "[ERROR] Verification failed: spine_app was able to create a table!" >&2
    psql -U "${ADMIN_USER}" -d "${DB_NAME}" -c "DROP TABLE IF EXISTS _unauthorized_test;"
    exit 1
else
    echo "[OK] Verified: spine_app is correctly forbidden from CREATE TABLE."
fi

echo "POSTGRES_ROLES_UPGRADE_SUCCESS"
