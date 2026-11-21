#!/usr/bin/env bash
# export_admin_token.sh — Supported secret-file handoff for host-side operational scripts
# Safely extracts the active administrative token from the running Spine API container
# or volume and writes it to a protected host path with 0600 permissions.
# Never echoes or exposes the token in stdout, stderr, or shell command history.

set -euo pipefail

TARGET_PATH="${1:-}"

if [ -z "${TARGET_PATH}" ]; then
  echo "Usage: $0 <HOST_OUTPUT_PATH>" >&2
  echo "Example: $0 ./certs/admin_token" >&2
  exit 1
fi

TARGET_DIR="$(dirname "${TARGET_PATH}")"
if [ ! -d "${TARGET_DIR}" ]; then
  mkdir -p "${TARGET_DIR}"
  chmod 700 "${TARGET_DIR}" 2>/dev/null || true
fi

# Touch target file first with 0600 permissions before populating content
touch "${TARGET_PATH}"
chmod 600 "${TARGET_PATH}" 2>/dev/null || true

DOCKER_BIN="docker"
if command -v docker.exe >/dev/null 2>&1; then
  DOCKER_BIN="docker.exe"
fi

CONTAINER_NAME="spine-forge-api"
if ! "${DOCKER_BIN}" ps --format '{{.Names}}' | grep -q "^${CONTAINER_NAME}$"; then
  echo "ERROR: Target container '${CONTAINER_NAME}' is not running." >&2
  echo "Start the production compose stack first or specify a valid token file." >&2
  exit 1
fi

if "${DOCKER_BIN}" exec "${CONTAINER_NAME}" cat /var/lib/spine/storage/admin_token > "${TARGET_PATH}" 2>/dev/null; then
  chmod 600 "${TARGET_PATH}" 2>/dev/null || true
  if [ -s "${TARGET_PATH}" ]; then
    echo "SUCCESS: Administrative token safely exported to: ${TARGET_PATH} (permissions: 0600)"
    exit 0
  fi
fi

echo "ERROR: Failed to extract admin token from container ${CONTAINER_NAME}:/var/lib/spine/storage/admin_token" >&2
rm -f "${TARGET_PATH}"
exit 1
