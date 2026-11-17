#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Backup Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail

SPINE_API_URL="${SPINE_API_URL:-http://127.0.0.1:3000}"
SPINE_ADMIN_TOKEN="${SPINE_ADMIN_TOKEN:-}"
BACKUP_ROOT="${BACKUP_ROOT:-/mnt/backups/spine}"

if [ -z "${SPINE_ADMIN_TOKEN}" ]; then
  echo "FATAL: SPINE_ADMIN_TOKEN must be set to acquire the cluster maintenance barrier." >&2
  exit 1
fi

echo "=== Phase 1: Acquiring Maintenance Barrier & Draining In-Flight Mutations ==="
ACQUIRE_RESP=$(curl -s -f -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/acquire" \
  -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" \
  -H "Content-Type: application/json" \
  -d '{"reason": "Scheduled Out-of-Band Multi-Volume Backup", "timeout_seconds": 300}')

echo "Maintenance barrier acquired: ${ACQUIRE_RESP}"

# Poll diagnostics until active_writers == 0
echo "Awaiting writer drain..."
DRAINED=0
GENERATION_TOKEN=""
for i in $(seq 1 30); do
  DIAG=$(curl -s -f "${SPINE_API_URL}/health/diagnostics")
  WRITERS=$(echo "${DIAG}" | grep -o '"active_writers":[0-9]*' | cut -d: -f2 || echo "999")
  IN_MAINT=$(echo "${DIAG}" | grep -o '"in_maintenance":true' || echo "")
  GENERATION_TOKEN=$(echo "${DIAG}" | grep -o '"current_fencing_generation":[0-9]*' | cut -d: -f2 || echo "")

  if [ "${IN_MAINT}" = '"in_maintenance":true' ] && [ "${WRITERS}" = "0" ] && [ -n "${GENERATION_TOKEN}" ]; then
    DRAINED=1
    break
  fi
  sleep 1
done

if [ ${DRAINED} -ne 1 ]; then
  echo "FATAL: Failed to drain in-flight writers within 30 seconds. Releasing barrier and aborting." >&2
  curl -s -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/release" \
    -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" || true
  exit 1
fi

echo "All active writers drained. Monotonic fencing generation token: ${GENERATION_TOKEN}"

TIMESTAMP=$(date -u +%Y%m%d_%H%M%SZ)
BACKUP_DIR="${BACKUP_ROOT}/${TIMESTAMP}_gen_${GENERATION_TOKEN}"
mkdir -p "${BACKUP_DIR}"

echo "=== Phase 2: Capturing Multi-Volume Snapshots to ${BACKUP_DIR} ==="

# Record fencing metadata
echo "${GENERATION_TOKEN}" > "${BACKUP_DIR}/FENCING_GENERATION"

# 1. Database snapshot
echo "Capturing PostgreSQL snapshot..."
docker exec spine-forge-postgres pg_dumpall -U spine_admin | gzip > "${BACKUP_DIR}/postgres_dump.sql.gz"

# 2. Coordinator state
echo "Capturing Coordinator consensus state..."
docker exec spine-coordinator tar -czf - -C /var/lib/spine . > "${BACKUP_DIR}/coordinator.tar.gz"

# 3. Storage node volumes
for NODE in 1 2 3; do
  echo "Capturing Storage Node ${NODE} volume..."
  docker exec "spine-storage-node-${NODE}" tar -czf - -C "/var/lib/spine/node-${NODE}" . > "${BACKUP_DIR}/storage_node_${NODE}.tar.gz"
done

# 4. API storage & host SSH keys
echo "Capturing API storage & SSH identities..."
docker exec spine-forge-api tar -czf - -C /var/lib/spine/ssh . > "${BACKUP_DIR}/spine_ssh.tar.gz"
docker exec spine-forge-api tar -czf - -C /var/lib/spine/storage . > "${BACKUP_DIR}/spine_storage.tar.gz"

# 5. Checksum manifest
echo "Generating SHA-256 checksum manifest..."
(cd "${BACKUP_DIR}" && sha256sum * > SHA256SUMS)

echo "=== Phase 3: Releasing Maintenance Barrier & Verifying Cluster Health ==="
curl -s -f -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/release" \
  -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}"

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
  echo "WARNING: Cluster not immediately healthy after maintenance release. Check logs." >&2
else
  echo "Cluster healthy and accepting requests."
fi

echo "Backup completed successfully at ${BACKUP_DIR}"
echo "Fencing Generation: ${GENERATION_TOKEN}"
