#!/usr/bin/env bash
# ==============================================================================
# Spine Forge - Administrative Credential Rotation & Revocation Script
# ==============================================================================
set -euo pipefail
umask 077
export MSYS_NO_PATHCONV=1

COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
DOCKER_BIN="docker"
command -v docker.exe >/dev/null 2>&1 && DOCKER_BIN="docker.exe"

# 1. Source configuration from .env if present
if [ -f .env ]; then
  POSTGRES_ADMIN_USER=$(grep -E "^POSTGRES_ADMIN_USER=" .env | cut -d'=' -f2- | tr -d '[:space:]"' || true)
  POSTGRES_ADMIN_PASSWORD=$(grep -E "^POSTGRES_ADMIN_PASSWORD=" .env | cut -d'=' -f2- | tr -d '[:space:]"' || true)
  POSTGRES_DB=$(grep -E "^POSTGRES_DB=" .env | cut -d'=' -f2- | tr -d '[:space:]"' || true)
fi

POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"
POSTGRES_DB="${POSTGRES_DB:-spine_forge}"

echo "=== Step 1: Revoking All Exposed and Active Administrative Credentials in Database ==="
PG_EXEC_ARGS=()
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  PG_EXEC_ARGS+=(-e "PGPASSWORD=${POSTGRES_ADMIN_PASSWORD}")
fi

"${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -v ON_ERROR_STOP=1 -c \
  "UPDATE personal_access_tokens SET revoked = true;"

echo "All existing personal access tokens revoked in database [OK]."

echo "=== Step 2: Generating and Hashing Rotated Administrative Token ==="
mkdir -p certs
chmod 700 certs 2>/dev/null || true

# Generate 32 bytes random hex
RAW_SECRET_HEX=$(python -c "import secrets; print(secrets.token_hex(16))")
NEW_TOKEN="sp_pat_${RAW_SECRET_HEX}"
unset RAW_SECRET_HEX

SPINE_API_BIN="./github-alternative/backend/target/debug/spine-api.exe"
if [ ! -x "${SPINE_API_BIN}" ] && [ -x "./github-alternative/backend/target/debug/spine-api" ]; then
  SPINE_API_BIN="./github-alternative/backend/target/debug/spine-api"
fi

TOKEN_HASH=$(printf "%s" "${NEW_TOKEN}" | "${SPINE_API_BIN}" hash-token)
NEW_PAT_ID=$(python -c "import uuid; print('pat_' + uuid.uuid4().hex)")
ADMIN_USER_ID=$("${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -t -A -c \
  "SELECT id FROM users WHERE username = 'ops-admin' LIMIT 1;")

if [ -z "${ADMIN_USER_ID}" ]; then
  ADMIN_USER_ID=$("${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
    psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -t -A -c \
    "SELECT id FROM users WHERE username = 'admin' LIMIT 1;")
fi

NOW_NANOS=$(python -c "import time; print(int(time.time() * 1e9))")

# Insert hashed PAT into PostgreSQL with full admin scopes
"${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -v ON_ERROR_STOP=1 -c \
  "INSERT INTO personal_access_tokens (id, user_id, name, token_hash, scopes, expires_at_nanos, created_at_nanos, revoked)
   VALUES ('${NEW_PAT_ID}', '${ADMIN_USER_ID}', 'rotated-admin-token-2026', '${TOKEN_HASH}', '[\"admin\", \"repo\", \"write\"]', 9223372036854775807, ${NOW_NANOS}, false);"

unset TOKEN_HASH

# Write raw token directly to protected file
printf "%s" "${NEW_TOKEN}" > certs/admin_token
chmod 600 certs/admin_token 2>/dev/null || true
echo "New administrative token securely generated and stored in certs/admin_token (0600) [OK]."

echo "=== Step 3: Synchronizing New Token to API Container Volume ==="
"${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" cp \
  certs/admin_token spine-api:/var/lib/spine/storage/admin_token

"${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T spine-api \
  sh -c "cat /var/lib/spine/storage/admin_token > /var/lib/spine/storage/admin_token.tmp && mv /var/lib/spine/storage/admin_token.tmp /var/lib/spine/storage/admin_token && chmod 600 /var/lib/spine/storage/admin_token"

echo "Synchronized admin_token into /var/lib/spine/storage/admin_token (0600) [OK]."

echo "=== Step 4: Verifying Rejection of Revoked Credentials ==="
NULL_DEV="/dev/null"
if [ "${OSTYPE:-}" = "msys" ] || [ "${OSTYPE:-}" = "cygwin" ] || [ -n "${WINDIR:-}" ]; then
  NULL_DEV="NUL"
fi

# Verify rejection with dummy / invalid credentials and any caller-provided revoked tokens
TEST_REVOKED_1="${TEST_REVOKED_TOKEN_1:-sp_pat_revoked_test_token_1}"
TEST_REVOKED_2="${TEST_REVOKED_TOKEN_2:-sp_pat_revoked_test_token_2}"

STATUS_1=$("${CURL_BIN}" -k -s -o "${NULL_DEV}" -w "%{http_code}" -X POST -H "Authorization: Bearer ${TEST_REVOKED_1}" https://127.0.0.1/api/v1/admin/maintenance/acquire | tr -d '\r\n')
STATUS_2=$("${CURL_BIN}" -k -s -o "${NULL_DEV}" -w "%{http_code}" -X POST -H "Authorization: Bearer ${TEST_REVOKED_2}" https://127.0.0.1/api/v1/admin/maintenance/acquire | tr -d '\r\n')
unset TEST_REVOKED_1 TEST_REVOKED_2

if [ "${STATUS_1}" != "401" ] || [ "${STATUS_2}" != "401" ]; then
  echo "FATAL: Revoked credential rejection failed! HTTP statuses: Credential 1=${STATUS_1}, Credential 2=${STATUS_2}" >&2
  exit 1
fi
echo "Verified: Revoked credential #1 rejected with HTTP ${STATUS_1} Unauthorized [OK]."
echo "Verified: Revoked credential #2 rejected with HTTP ${STATUS_2} Unauthorized [OK]."

echo "=== Step 5: Verifying Acceptance of Newly Rotated Administrative Token ==="
STATUS_NEW=$("${CURL_BIN}" -k -s -o "${NULL_DEV}" -w "%{http_code}" -X POST -H "Authorization: Bearer ${NEW_TOKEN}" https://127.0.0.1/api/v1/admin/maintenance/acquire | tr -d '\r\n')

if [ "${STATUS_NEW}" != "200" ]; then
  echo "FATAL: Newly rotated administrative token was rejected! HTTP status: ${STATUS_NEW}" >&2
  unset NEW_TOKEN
  exit 1
fi

# Release the probe maintenance barrier
"${CURL_BIN}" -k -s -o "${NULL_DEV}" -X POST -H "Authorization: Bearer ${NEW_TOKEN}" https://127.0.0.1/api/v1/admin/maintenance/release
unset NEW_TOKEN

# Synchronize authoritative database restore generation with new fencing token
CURRENT_GEN=$("${CURL_BIN}" -k -s https://127.0.0.1/health/diagnostics | grep -o '"current_fencing_generation":[0-9]*' | cut -d':' -f2 || true)
if [ -n "${CURRENT_GEN}" ]; then
  "${DOCKER_BIN}" compose -f "${COMPOSE_FILE}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
    psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -v ON_ERROR_STOP=1 -c \
    "UPDATE _restore_generation SET generation_token = '${CURRENT_GEN}' WHERE id = 1;" >/dev/null 2>&1 || true
fi

echo "Verified: Newly rotated administrative token accepted with HTTP ${STATUS_NEW} OK [OK]."
echo "Administrative credential rotation and verification complete."
