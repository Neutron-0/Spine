#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Backup Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail

SPINE_API_URL="${SPINE_API_URL:-http://127.0.0.1:3000}"
SPINE_ADMIN_TOKEN="${SPINE_ADMIN_TOKEN:-}"
BACKUP_ROOT="${BACKUP_ROOT:-/mnt/backups/spine}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"

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
mkdir -p "${BACKUP_DIR}"

echo "=== Phase 2: Capturing Multi-Volume Snapshots to ${BACKUP_DIR} ==="

# Record fencing metadata for global and individual participants
echo "${GENERATION_TOKEN}" > "${BACKUP_DIR}/FENCING_GENERATION"
echo "${COORD_GEN}" > "${BACKUP_DIR}/FENCING_GENERATION_COORDINATOR"
echo "${NODE_1_GEN}" > "${BACKUP_DIR}/FENCING_GENERATION_NODE_1"
echo "${NODE_2_GEN}" > "${BACKUP_DIR}/FENCING_GENERATION_NODE_2"
echo "${NODE_3_GEN}" > "${BACKUP_DIR}/FENCING_GENERATION_NODE_3"
echo "${API_GENERATION}" > "${BACKUP_DIR}/FENCING_GENERATION_API"

cat <<EOF > "${BACKUP_DIR}/PARTICIPANT_GENERATION_AUDIT.json"
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
    pg_dumpall -U "${POSTGRES_ADMIN_USER}" | gzip > "${BACKUP_DIR}/postgres_dump.sql.gz"
else
  docker compose -f "${COMPOSE_FILE}" exec -T postgres \
    pg_dumpall -U "${POSTGRES_ADMIN_USER}" | gzip > "${BACKUP_DIR}/postgres_dump.sql.gz"
fi

# 2. Coordinator state
echo "Capturing Coordinator consensus state..."
docker compose -f "${COMPOSE_FILE}" exec -T coordinator \
  tar -czf - -C /var/lib/spine . > "${BACKUP_DIR}/coordinator.tar.gz"

# 3. Storage node volumes
for NODE in 1 2 3; do
  echo "Capturing Storage Node ${NODE} volume..."
  docker compose -f "${COMPOSE_FILE}" exec -T "storage-node-${NODE}" \
    tar -czf - -C "/var/lib/spine/node-${NODE}" . > "${BACKUP_DIR}/storage_node_${NODE}.tar.gz"
done

# 4. API storage & host SSH keys
echo "Capturing API storage & SSH identities..."
docker compose -f "${COMPOSE_FILE}" exec -T spine-api \
  tar -czf - -C /var/lib/spine/ssh . > "${BACKUP_DIR}/spine_ssh.tar.gz"
docker compose -f "${COMPOSE_FILE}" exec -T spine-api \
  tar -czf - -C /var/lib/spine/storage . > "${BACKUP_DIR}/spine_storage.tar.gz"

# 5. Checksum manifest
echo "Generating SHA-256 checksum manifest..."
(cd "${BACKUP_DIR}" && sha256sum * > SHA256SUMS)

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
