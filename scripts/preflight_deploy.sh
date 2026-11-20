#!/usr/bin/env bash
# ==============================================================================
# Spine Forge - Production Deployment Preflight Validator
# Validates environment secrets, TLS certificate readability, and Compose syntax
# ==============================================================================

set -euo pipefail

ENV_FILE="${1:-.env}"
COMPOSE_FILE="${2:-docker-compose.prod.yml}"

echo "=================================================================="
echo " Spine Forge - Production Deployment Preflight Check"
echo "=================================================================="
echo "Target Environment File: ${ENV_FILE}"
echo "Target Compose File:     ${COMPOSE_FILE}"
echo "------------------------------------------------------------------"

FAILED=0

# 1. Verify environment file exists
if [ ! -f "${ENV_FILE}" ]; then
  echo "[FAIL] Environment configuration file '${ENV_FILE}' not found." >&2
  echo "       Copy .env.example to .env and configure production secrets." >&2
  exit 1
fi

echo "[PASS] Environment configuration file exists: ${ENV_FILE}"

# Load environment file safely
set -a
# shellcheck disable=SC1090
source "${ENV_FILE}"
set +a

# 2. Check required secret variables
check_secret() {
  local var_name="$1"
  local min_len="$2"
  local val="${!var_name:-}"

  if [ -z "${val}" ]; then
    echo "[FAIL] Required secret '${var_name}' is unset or empty." >&2
    FAILED=1
  elif [ "${#val}" -lt "${min_len}" ]; then
    echo "[FAIL] Secret '${var_name}' is too short (${#val} chars; minimum ${min_len} required)." >&2
    FAILED=1
  else
    echo "[PASS] Secret '${var_name}' meets minimum length (${min_len}+ chars)."
  fi
}

check_secret "POSTGRES_ADMIN_PASSWORD" 16
check_secret "POSTGRES_APP_PASSWORD" 16
check_secret "POSTGRES_MIGRATOR_PASSWORD" 16
check_secret "SPINE_AUTH_SECRET_KEY" 32
check_secret "SPINE_LFS_SECRET" 32

# Verify backup encryption passphrase (either raw secret or secure mounted file)
if [ -n "${SPINE_BACKUP_PASSPHRASE:-}" ]; then
  check_secret "SPINE_BACKUP_PASSPHRASE" 16
elif [ -n "${SPINE_BACKUP_PASSPHRASE_FILE:-}" ]; then
  if [ -f "${SPINE_BACKUP_PASSPHRASE_FILE}" ] && [ -r "${SPINE_BACKUP_PASSPHRASE_FILE}" ]; then
    echo "[PASS] Backup passphrase file verified readable: ${SPINE_BACKUP_PASSPHRASE_FILE}"
  else
    echo "[FAIL] Backup passphrase file '${SPINE_BACKUP_PASSPHRASE_FILE}' not found or unreadable." >&2
    FAILED=1
  fi
else
  echo "[FAIL] Either SPINE_BACKUP_PASSPHRASE or SPINE_BACKUP_PASSPHRASE_FILE must be configured." >&2
  FAILED=1
fi

# 3. Check TLS certificate and private key paths
CERT_FILE="${SPINE_TLS_CERT_FILE:-}"
KEY_FILE="${SPINE_TLS_KEY_FILE:-}"

if [ -z "${CERT_FILE}" ]; then
  echo "[FAIL] SPINE_TLS_CERT_FILE is unset in ${ENV_FILE}." >&2
  FAILED=1
elif [ ! -f "${CERT_FILE}" ]; then
  echo "[FAIL] TLS certificate file '${CERT_FILE}' does not exist." >&2
  echo "       Provision a valid TLS certificate before booting edge ingress." >&2
  FAILED=1
elif [ ! -r "${CERT_FILE}" ]; then
  echo "[FAIL] TLS certificate file '${CERT_FILE}' is not readable." >&2
  FAILED=1
elif [ ! -s "${CERT_FILE}" ]; then
  echo "[FAIL] TLS certificate file '${CERT_FILE}' is empty (0 bytes)." >&2
  FAILED=1
else
  echo "[PASS] TLS certificate file verified readable: ${CERT_FILE}"
fi

if [ -z "${KEY_FILE}" ]; then
  echo "[FAIL] SPINE_TLS_KEY_FILE is unset in ${ENV_FILE}." >&2
  FAILED=1
elif [ ! -f "${KEY_FILE}" ]; then
  echo "[FAIL] TLS private key file '${KEY_FILE}' does not exist." >&2
  echo "       Provision the matching TLS private key before booting edge ingress." >&2
  FAILED=1
elif [ ! -r "${KEY_FILE}" ]; then
  echo "[FAIL] TLS private key file '${KEY_FILE}' is not readable." >&2
  FAILED=1
elif [ ! -s "${KEY_FILE}" ]; then
  echo "[FAIL] TLS private key file '${KEY_FILE}' is empty (0 bytes)." >&2
  FAILED=1
else
  echo "[PASS] TLS private key file verified readable: ${KEY_FILE}"
fi

# 4. Check Compose specification syntax
if command -v docker >/dev/null 2>&1 && docker compose version >/dev/null 2>&1; then
  echo "Validating Docker Compose interpolation..."
  if docker compose --env-file "${ENV_FILE}" -f "${COMPOSE_FILE}" config --quiet >/dev/null 2>&1; then
    echo "[PASS] Docker Compose configuration interpolated cleanly."
  else
    echo "[FAIL] Docker Compose configuration validation failed for '${COMPOSE_FILE}'." >&2
    docker compose --env-file "${ENV_FILE}" -f "${COMPOSE_FILE}" config || true
    FAILED=1
  fi
else
  echo "[WARN] Docker Compose CLI not detected; skipping compose config validation."
fi

echo "------------------------------------------------------------------"
if [ "${FAILED}" -ne 0 ]; then
  echo "Preflight check FAILED. Correct configuration errors above before deploying." >&2
  exit 1
fi

echo "Preflight check PASSED. Cluster configuration is ready for deployment."
exit 0
