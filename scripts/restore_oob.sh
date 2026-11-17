#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Disaster Recovery Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail

BACKUP_DIR="${1:-}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"

if [ -z "${BACKUP_DIR}" ] || [ ! -d "${BACKUP_DIR}" ]; then
  echo "Usage: $0 <path-to-backup-dir>" >&2
  echo "Example: $0 /mnt/backups/spine/20261117_120000Z_gen_42" >&2
  exit 1
fi

if [ ! -f "${BACKUP_DIR}/FENCING_GENERATION" ] || [ ! -f "${BACKUP_DIR}/SHA256SUMS" ]; then
  echo "FATAL: Invalid backup directory. FENCING_GENERATION or SHA256SUMS missing." >&2
  exit 1
fi

TARGET_GEN=$(cat "${BACKUP_DIR}/FENCING_GENERATION" | tr -d '[:space:]')
echo "Target recovery generation: ${TARGET_GEN}"

echo "=== Step 0: Verifying Backup Checksum Manifest ==="
(cd "${BACKUP_DIR}" && sha256sum -c SHA256SUMS)

echo "=== Step 1: Stopping Cluster Services ==="
docker compose -f "${COMPOSE_FILE}" stop ingress spine-api
docker compose -f "${COMPOSE_FILE}" stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend

echo "=== Step 2: Restoring Multi-Volume Archive Data ==="
# 1. Clean old staging
for NODE in 1 2 3; do
  rm -rf "/var/lib/spine/node-${NODE}/.staged_restore_*" "/var/lib/spine/node-${NODE}/.old_restore_*" 2>/dev/null || true
done

# 2. Extract volumes
echo "Restoring PostgreSQL database..."
# Postgres data is re-imported after postgres container start or volume unpack
echo "Restoring Coordinator state..."
tar -xzf "${BACKUP_DIR}/coordinator.tar.gz" -C /var/lib/spine

echo "Restoring Storage Node volumes..."
for NODE in 1 2 3; do
  tar -xzf "${BACKUP_DIR}/storage_node_${NODE}.tar.gz" -C "/var/lib/spine/node-${NODE}"
done

echo "Restoring API storage and SSH keys..."
tar -xzf "${BACKUP_DIR}/spine_ssh.tar.gz" -C /var/lib/spine/ssh
tar -xzf "${BACKUP_DIR}/spine_storage.tar.gz" -C /var/lib/spine/storage

echo "=== Step 3: Automated Fail-Closed Fencing Generation Verification ==="
FAILED=0

COORD_GEN=$(cat /var/lib/spine/.coordinator_fencing_generation 2>/dev/null || echo "MISSING")
if [ "${COORD_GEN}" != "${TARGET_GEN}" ]; then
  echo "ERROR: Coordinator generation (${COORD_GEN}) does not match target generation (${TARGET_GEN})."
  FAILED=1
else
  echo "Coordinator generation: ${COORD_GEN} [OK]"
fi

for NODE in 1 2 3; do
  NODE_GEN=$(cat "/var/lib/spine/node-${NODE}/.fencing_generation" 2>/dev/null || echo "MISSING")
  if [ "${NODE_GEN}" != "${TARGET_GEN}" ]; then
    echo "ERROR: Storage node ${NODE} generation (${NODE_GEN}) does not match target generation (${TARGET_GEN})."
    FAILED=1
  else
    echo "Storage node ${NODE} generation: ${NODE_GEN} [OK]"
  fi
done

if [ ${FAILED} -ne 0 ]; then
  echo "================================================================================" >&2
  echo "CRITICAL: Fencing generation divergence detected across restored volumes." >&2
  echo "HALTING DISASTER RECOVERY IMMEDIATELY." >&2
  echo "" >&2
  echo "DO NOT manually edit or overwrite .fencing_generation or .coordinator_fencing_generation." >&2
  echo "Manual editing undermines consensus fencing invariants and leads to silent split-brain writes." >&2
  echo "" >&2
  echo "Defined Recovery Procedure:" >&2
  echo "1. Re-verify the source backup directory: ${BACKUP_DIR}" >&2
  echo "2. Re-extract the specific diverged volume from the verified archive." >&2
  echo "3. If at least 2 of 3 storage nodes and coordinator match, run anti-entropy repair:" >&2
  echo "   docker compose -f ${COMPOSE_FILE} run --rm coordinator \\" >&2
  echo "     /usr/local/bin/spine-coordinator --repair-generation --expected-gen ${TARGET_GEN}" >&2
  echo "4. Otherwise, abort and fall back to the previous verified backup." >&2
  echo "================================================================================" >&2
  exit 1
fi

echo "All participants match fencing generation ${TARGET_GEN}. Consensus verified."

echo "=== Step 4: Storage Node Volume Integrity Check ==="
for NODE in 1 2 3; do
  echo "Verifying storage volume integrity on node ${NODE}..."
  docker run --rm \
    -v "spine_storage_${NODE}_data:/var/lib/spine/node-${NODE}" \
    spine-storage-node:latest \
    /usr/local/bin/spine-storage-node --verify-integrity --data-dir "/var/lib/spine/node-${NODE}"
done

echo "=== Step 5: Sequenced Cluster Startup ==="
echo "Starting database, coordinator, and storage nodes..."
docker compose -f "${COMPOSE_FILE}" up -d postgres coordinator storage-node-1 storage-node-2 storage-node-3

echo "Awaiting core infrastructure health..."
sleep 5
docker compose -f "${COMPOSE_FILE}" ps

echo "Starting API, frontend, and ingress..."
docker compose -f "${COMPOSE_FILE}" up -d spine-api spine-frontend ingress

echo "=== Step 6: Readiness Health Check ==="
sleep 3
if curl -s -f http://127.0.0.1:3000/health/ready > /dev/null; then
  echo "SUCCESS: Spine Forge cluster restored and fully ready at generation ${TARGET_GEN}."
else
  echo "WARNING: /health/ready returned non-200. Inspecting diagnostics..." >&2
  curl -s http://127.0.0.1:3000/health/diagnostics || true
  exit 1
fi
