#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Disaster Recovery Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail

BACKUP_DIR="${1:-}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"

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
docker compose -f "${COMPOSE_FILE}" stop ingress spine-api || true
docker compose -f "${COMPOSE_FILE}" stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend || true

echo "=== Step 2: Restoring Multi-Volume Archive Data into Docker Volumes ==="

# 1. Clean old ephemeral staging directories in storage nodes
for NODE in 1 2 3; do
  echo "Cleaning staging in storage node ${NODE}..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/.staged_restore_* /var/lib/spine/node-${NODE}/.old_restore_* 2>/dev/null || true"
done

# 2. Extract volumes directly via container volume mounts
echo "Restoring Coordinator state into coordinator_data volume..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh coordinator \
  -c "tar -xzf - -C /var/lib/spine" < "${BACKUP_DIR}/coordinator.tar.gz"

for NODE in 1 2 3; do
  echo "Restoring Storage Node ${NODE} into storage_${NODE}_data volume..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "tar -xzf - -C /var/lib/spine/node-${NODE}" < "${BACKUP_DIR}/storage_node_${NODE}.tar.gz"
done

echo "Restoring API SSH identities into spine_ssh volume..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "tar -xzf - -C /var/lib/spine/ssh" < "${BACKUP_DIR}/spine_ssh.tar.gz"

echo "Restoring API storage cache into spine_storage volume..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "tar -xzf - -C /var/lib/spine/storage" < "${BACKUP_DIR}/spine_storage.tar.gz"

echo "=== Step 3: Automated Fail-Closed Fencing Generation Verification ==="
FAILED=0

COORD_GEN=$(docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint cat coordinator \
  /var/lib/spine/.coordinator_fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")

if [ "${COORD_GEN}" != "${TARGET_GEN}" ]; then
  echo "ERROR: Coordinator generation (${COORD_GEN}) does not match target generation (${TARGET_GEN})."
  FAILED=1
else
  echo "Coordinator generation: ${COORD_GEN} [OK]"
fi

for NODE in 1 2 3; do
  NODE_GEN=$(docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint cat "storage-node-${NODE}" \
    "/var/lib/spine/node-${NODE}/.fencing_generation" 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
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
  echo "     repair-generation --expected-gen ${TARGET_GEN}" >&2
  echo "   (or flag syntax: --repair-generation --expected-gen ${TARGET_GEN})" >&2
  echo "4. Otherwise, abort and fall back to the previous verified backup." >&2
  echo "================================================================================" >&2
  exit 1
fi

echo "All participants match fencing generation ${TARGET_GEN}. Consensus verified."

echo "=== Step 4: Storage Node Volume Integrity Check ==="
for NODE in 1 2 3; do
  echo "Verifying storage volume integrity on node ${NODE}..."
  docker compose -f "${COMPOSE_FILE}" run --rm "storage-node-${NODE}" \
    /usr/local/bin/spine-storage-node --verify-integrity --data-dir "/var/lib/spine/node-${NODE}"
done

echo "=== Step 5: Sequenced Cluster Startup & Database Restoration ==="
echo "Starting PostgreSQL container..."
docker compose -f "${COMPOSE_FILE}" up -d postgres

echo "Awaiting PostgreSQL readiness..."
READY_PG=0
for i in $(seq 1 30); do
  if docker compose -f "${COMPOSE_FILE}" exec -T postgres pg_isready -U "${POSTGRES_ADMIN_USER}" >/dev/null 2>&1; then
    READY_PG=1
    break
  fi
  sleep 1
done

if [ ${READY_PG} -ne 1 ]; then
  echo "FATAL: PostgreSQL failed to become ready for database restoration." >&2
  exit 1
fi

echo "Importing PostgreSQL database from ${BACKUP_DIR}/postgres_dump.sql.gz..."
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  gzip -dc "${BACKUP_DIR}/postgres_dump.sql.gz" | \
    docker compose -f "${COMPOSE_FILE}" exec -T -e PGPASSWORD="${POSTGRES_ADMIN_PASSWORD}" postgres \
    psql -U "${POSTGRES_ADMIN_USER}" -d postgres
else
  gzip -dc "${BACKUP_DIR}/postgres_dump.sql.gz" | \
    docker compose -f "${COMPOSE_FILE}" exec -T postgres \
    psql -U "${POSTGRES_ADMIN_USER}" -d postgres
fi
echo "PostgreSQL restoration complete."

echo "Starting coordinator and storage nodes..."
docker compose -f "${COMPOSE_FILE}" up -d coordinator storage-node-1 storage-node-2 storage-node-3

echo "Awaiting core infrastructure health..."
sleep 5
docker compose -f "${COMPOSE_FILE}" ps

echo "Starting API, frontend, and ingress..."
docker compose -f "${COMPOSE_FILE}" up -d spine-api spine-frontend ingress

echo "=== Step 6: Readiness Health Check ==="
sleep 3
READY_APP=0
for i in $(seq 1 30); do
  if curl -s -f http://127.0.0.1:3000/health/ready > /dev/null 2>&1; then
    READY_APP=1
    break
  fi
  sleep 2
done

if [ ${READY_APP} -eq 1 ]; then
  echo "SUCCESS: Spine Forge cluster restored and fully ready at generation ${TARGET_GEN}."
else
  echo "WARNING: /health/ready returned non-200. Inspecting diagnostics..." >&2
  curl -s http://127.0.0.1:3000/health/diagnostics || true
  exit 1
fi
