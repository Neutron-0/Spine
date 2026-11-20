#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Backup Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail
umask 077

SPINE_API_URL="${SPINE_API_URL:-http://127.0.0.1:3000}"
SPINE_ADMIN_TOKEN="${SPINE_ADMIN_TOKEN:-}"
BACKUP_ROOT="${BACKUP_ROOT:-/mnt/backups/spine}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"
SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE:-}"
SPINE_BACKUP_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE:-}"

if [ -z "${SPINE_BACKUP_PASSPHRASE}" ] && [ -n "${SPINE_BACKUP_PASSPHRASE_FILE}" ] && [ -f "${SPINE_BACKUP_PASSPHRASE_FILE}" ]; then
  SPINE_BACKUP_PASSPHRASE=$(cat "${SPINE_BACKUP_PASSPHRASE_FILE}")
fi

if [ -z "${SPINE_BACKUP_PASSPHRASE}" ]; then
  echo "FATAL: Authenticated encryption is mandatory for production backups." >&2
  echo "Please set SPINE_BACKUP_PASSPHRASE or provide SPINE_BACKUP_PASSPHRASE_FILE." >&2
  exit 1
fi

if [ -z "${SPINE_ADMIN_TOKEN}" ]; then
  echo "FATAL: SPINE_ADMIN_TOKEN must be set to acquire the cluster maintenance barrier." >&2
  exit 1
fi

RELEASED=0
cleanup_barrier() {
  if [ "${RELEASED}" -eq 0 ] && [ -n "${SPINE_ADMIN_TOKEN:-}" ]; then
    echo "NOTICE: Cleaning up — ensuring cluster maintenance barrier is released..." >&2
    curl -s -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/release" \
      -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" > /dev/null 2>&1 || true
  fi
}
trap cleanup_barrier EXIT ERR INT TERM

echo "=== Phase 1: Acquiring Maintenance Barrier & Draining In-Flight Mutations ==="
ACQUIRE_RESP=$(curl -s -f -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/acquire" \
  -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"reason": "Scheduled Out-of-Band Multi-Volume Backup", "timeout_seconds": 300}')

echo "Maintenance barrier acquired: ${ACQUIRE_RESP}"

# Poll diagnostics until active_writers == 0
echo "Awaiting writer drain..."
DRAINED=0
API_GENERATION=""
for i in $(seq 1 30); do
  DIAG=$(curl -s -f "${SPINE_API_URL}/health/diagnostics")
  WRITERS=$(echo "${DIAG}" | grep -o '"active_writers":[0-9]*' | cut -d: -f2 || echo "999")
  IN_MAINT=$(echo "${DIAG}" | grep -o '"in_maintenance":true' || echo "")
  API_GENERATION=$(echo "${DIAG}" | grep -o '"current_fencing_generation":[0-9]*' | cut -d: -f2 || echo "")

  if [ "${IN_MAINT}" = '"in_maintenance":true' ] && [ "${WRITERS}" = "0" ] && [ -n "${API_GENERATION}" ]; then
    DRAINED=1
    break
  fi
  sleep 1
done

if [ ${DRAINED} -ne 1 ]; then
  echo "FATAL: Failed to drain in-flight writers within 30 seconds. Releasing barrier and aborting." >&2
  exit 1
fi

echo "All active writers drained. API recorded generation: ${API_GENERATION}"

# Phase 1b: Direct Pre-Capture Audit Across All Distributed Participants
echo "=== Phase 1b: Direct Participant Fencing Generation Verification ==="
COORD_GEN=$(docker compose -f "${COMPOSE_FILE}" exec -T coordinator \
  cat /var/lib/spine/.coordinator_fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
NODE_1_GEN=$(docker compose -f "${COMPOSE_FILE}" exec -T storage-node-1 \
  cat /var/lib/spine/node-1/.fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
NODE_2_GEN=$(docker compose -f "${COMPOSE_FILE}" exec -T storage-node-2 \
  cat /var/lib/spine/node-2/.fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
NODE_3_GEN=$(docker compose -f "${COMPOSE_FILE}" exec -T storage-node-3 \
  cat /var/lib/spine/node-3/.fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")

echo "Participant generations:"
echo "  Control Plane API: ${API_GENERATION}"
echo "  Coordinator:       ${COORD_GEN}"
echo "  Storage Node 1:    ${NODE_1_GEN}"
echo "  Storage Node 2:    ${NODE_2_GEN}"
echo "  Storage Node 3:    ${NODE_3_GEN}"

MISMATCH=0
if [ "${COORD_GEN}" != "${API_GENERATION}" ]; then
  echo "ERROR: Coordinator generation (${COORD_GEN}) diverges from API (${API_GENERATION})" >&2
  MISMATCH=1
fi
if [ "${NODE_1_GEN}" != "${API_GENERATION}" ]; then
  echo "ERROR: Storage node 1 generation (${NODE_1_GEN}) diverges from API (${API_GENERATION})" >&2
  MISMATCH=1
fi
if [ "${NODE_2_GEN}" != "${API_GENERATION}" ]; then
  echo "ERROR: Storage node 2 generation (${NODE_2_GEN}) diverges from API (${API_GENERATION})" >&2
  MISMATCH=1
fi
if [ "${NODE_3_GEN}" != "${API_GENERATION}" ]; then
  echo "ERROR: Storage node 3 generation (${NODE_3_GEN}) diverges from API (${API_GENERATION})" >&2
  MISMATCH=1
fi

if [ ${MISMATCH} -ne 0 ]; then
  echo "FATAL: Pre-capture audit detected generation divergence across participants." >&2
  echo "Aborting backup to prevent capturing inconsistent snapshots. Releasing barrier." >&2
  exit 1
fi

GENERATION_TOKEN="${API_GENERATION}"
echo "Unanimous fencing generation token verified: ${GENERATION_TOKEN}"

TIMESTAMP=$(date -u +%Y%m%d_%H%M%SZ)
BACKUP_DIR="${BACKUP_ROOT}/${TIMESTAMP}_gen_${GENERATION_TOKEN}"
mkdir -p "${BACKUP_ROOT}"
chmod 700 "${BACKUP_ROOT}" 2>/dev/null || true
mkdir -p "${BACKUP_DIR}"
chmod 700 "${BACKUP_DIR}"

# Fail-closed permission check on newly created backup directory
DIR_MODE=$(stat -c "%a" "${BACKUP_DIR}" 2>/dev/null || stat -f "%Lp" "${BACKUP_DIR}" 2>/dev/null || echo "")
if [ -n "${DIR_MODE}" ] && [ "${DIR_MODE}" != "700" ]; then
  echo "FATAL: Insecure permissions detected on backup directory ${BACKUP_DIR}: mode is ${DIR_MODE}, expected 700." >&2
  exit 1
fi

echo "=== Phase 2: Capturing Multi-Volume Snapshots to ${BACKUP_DIR} ==="
STAGING_DIR="${BACKUP_DIR}/.staging"
mkdir -p "${STAGING_DIR}"
chmod 700 "${STAGING_DIR}"

# Record fencing metadata for global and individual participants
echo "${GENERATION_TOKEN}" > "${STAGING_DIR}/FENCING_GENERATION"
echo "${COORD_GEN}" > "${STAGING_DIR}/FENCING_GENERATION_COORDINATOR"
echo "${NODE_1_GEN}" > "${STAGING_DIR}/FENCING_GENERATION_NODE_1"
echo "${NODE_2_GEN}" > "${STAGING_DIR}/FENCING_GENERATION_NODE_2"
echo "${NODE_3_GEN}" > "${STAGING_DIR}/FENCING_GENERATION_NODE_3"
echo "${API_GENERATION}" > "${STAGING_DIR}/FENCING_GENERATION_API"

cat <<EOF > "${STAGING_DIR}/PARTICIPANT_GENERATION_AUDIT.json"
{
  "timestamp_utc": "${TIMESTAMP}",
  "consensus_generation": ${GENERATION_TOKEN},
  "participants": {
    "api": ${API_GENERATION},
    "coordinator": ${COORD_GEN},
    "storage_node_1": ${NODE_1_GEN},
    "storage_node_2": ${NODE_2_GEN},
    "storage_node_3": ${NODE_3_GEN}
  },
  "active_writers": 0,
  "unanimous": true
}
EOF

# 1. Database snapshot via docker compose exec
echo "Capturing PostgreSQL dump..."
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  docker compose -f "${COMPOSE_FILE}" exec -T -e PGPASSWORD="${POSTGRES_ADMIN_PASSWORD}" postgres \
    pg_dumpall -U "${POSTGRES_ADMIN_USER}" | gzip > "${STAGING_DIR}/postgres_dump.sql.gz"
else
  docker compose -f "${COMPOSE_FILE}" exec -T postgres \
    pg_dumpall -U "${POSTGRES_ADMIN_USER}" | gzip > "${STAGING_DIR}/postgres_dump.sql.gz"
fi

# 2. Coordinator state
echo "Capturing Coordinator consensus state..."
docker compose -f "${COMPOSE_FILE}" exec -T coordinator \
  tar -czf - -C /var/lib/spine . > "${STAGING_DIR}/coordinator.tar.gz"

# 3. Storage node volumes
for NODE in 1 2 3; do
  echo "Capturing Storage Node ${NODE} volume..."
  docker compose -f "${COMPOSE_FILE}" exec -T "storage-node-${NODE}" \
    tar -czf - -C "/var/lib/spine/node-${NODE}" . > "${STAGING_DIR}/storage_node_${NODE}.tar.gz"
done

# 4. API storage & host SSH keys
echo "Capturing API storage & SSH identities..."
docker compose -f "${COMPOSE_FILE}" exec -T spine-api \
  tar -czf - -C /var/lib/spine/ssh . > "${STAGING_DIR}/spine_ssh.tar.gz"
docker compose -f "${COMPOSE_FILE}" exec -T spine-api \
  tar -czf - -C /var/lib/spine/storage . > "${STAGING_DIR}/spine_storage.tar.gz"

# 5. Internal checksum manifest of captured participants
echo "Generating internal SHA-256 checksum manifest..."
(cd "${STAGING_DIR}" && sha256sum * > SHA256SUMS)

# 6. Authenticated AEAD Encryption (SPNEBAK v2)
echo "=== Phase 2b: Packaging & Authenticated AEAD Encryption (SPNEBAK v2) ==="
ENCRYPTED_ARCHIVE="${BACKUP_DIR}/backup.spnebak"
PLAINTEXT_TAR="${BACKUP_DIR}/.plaintext_bundle.tar.gz"

echo "Bundling captured snapshot artifacts into temporary bundle..."
tar -czf "${PLAINTEXT_TAR}" -C "${STAGING_DIR}" .

echo "Encrypting backup bundle into SPNEBAK v2 AEAD envelope (AES-256-GCM)..."
if command -v spine-api >/dev/null 2>&1; then
  SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" \
    spine-api backup encrypt --input "${PLAINTEXT_TAR}" --output "${ENCRYPTED_ARCHIVE}"
else
  # Encrypt through the spine-api container
  docker compose -f "${COMPOSE_FILE}" exec -T \
    -e SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" spine-api \
    spine-api backup encrypt --input - --output - < "${PLAINTEXT_TAR}" > "${ENCRYPTED_ARCHIVE}"
fi

# Verify the encrypted archive is valid SPNEBAK v2 format
if [ ! -s "${ENCRYPTED_ARCHIVE}" ]; then
  echo "FATAL: Encrypted backup archive was not created or is zero-byte empty!" >&2
  exit 1
fi

MAGIC_CHECK=$(dd if="${ENCRYPTED_ARCHIVE}" bs=1 count=8 2>/dev/null || true)
if [ "${MAGIC_CHECK}" != "SPNEBAK"$'\x02' ]; then
  echo "FATAL: Verification failed: ${ENCRYPTED_ARCHIVE} does not start with SPNEBAK v2 magic!" >&2
  exit 1
fi

echo "Encrypted archive verified: ${ENCRYPTED_ARCHIVE} (SPNEBAK v2 AEAD envelope confirmed)."

# STRICT DATA HYGIENE: Securely purge all unencrypted plaintext archives and database dumps!
echo "Purging unencrypted staging artifacts from disk..."
rm -rf "${PLAINTEXT_TAR}" "${STAGING_DIR}"

# Generate checksum of final encrypted backup archive
(cd "${BACKUP_DIR}" && sha256sum backup.spnebak > SHA256SUMS)

# Enforce restrictive permissions on the encrypted archive and manifest
echo "Enforcing restrictive permissions (0600 files, 0700 dir) on backup artifacts..."
chmod 600 "${BACKUP_DIR}"/*
chmod 700 "${BACKUP_DIR}"

echo "=== Phase 3: Releasing Maintenance Barrier & Verifying Cluster Health ==="
curl -s -f -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/release" \
  -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}"
RELEASED=1

# Verify health
echo "Awaiting cluster ready state..."
READY=0
for i in $(seq 1 15); do
  if curl -s -f "${SPINE_API_URL}/health/ready" > /dev/null; then
    READY=1
    break
  fi
  sleep 1
done

if [ ${READY} -ne 1 ]; then
  echo "WARNING: Cluster not immediately healthy after maintenance release. Check diagnostics." >&2
  curl -s "${SPINE_API_URL}/health/diagnostics" || true
else
  echo "Cluster healthy and accepting requests."
fi

echo "Backup completed successfully at ${BACKUP_DIR}"
echo "Unanimous Fencing Generation: ${GENERATION_TOKEN}"
