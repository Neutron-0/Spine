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

echo "=== Step 1: Stopping Cluster Services & Verifying Full Quiescence ==="
# Stop services; failures here are fatal (no || true)
docker compose -f "${COMPOSE_FILE}" stop ingress spine-api
docker compose -f "${COMPOSE_FILE}" stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend

# Strictly verify no stateful containers are running
RUNNING=$(docker compose -f "${COMPOSE_FILE}" ps --services --filter "status=running")
if [ -n "${RUNNING}" ]; then
  echo "FATAL: The following services failed to stop and are still running:" >&2
  echo "${RUNNING}" >&2
  echo "Aborting restore to prevent data corruption." >&2
  exit 1
fi
echo "All cluster services confirmed stopped."

# Set up external rollback directory outside container volume paths
ROLLBACK_DIR="${BACKUP_DIR}/.pre_restore_rollback_$(date +%s)"
mkdir -p "${ROLLBACK_DIR}"

RESTORE_SUCCESS=0
rollback_volumes() {
  if [ "${RESTORE_SUCCESS}" -eq 0 ] && [ -d "${ROLLBACK_DIR:-}" ]; then
    echo "================================================================================" >&2
    echo "CRITICAL: Restore failed or was aborted before commit. EXECUTING AUTOMATIC ROLLBACK." >&2
    echo "Reverting all cluster volumes to pre-restore state..." >&2

    # Stop any partially started services
    docker compose -f "${COMPOSE_FILE}" stop ingress spine-api storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend >/dev/null 2>&1 || true

    # Roll back Coordinator
    if [ -f "${ROLLBACK_DIR}/coordinator.tar.gz" ]; then
      echo "Rolling back Coordinator volume..." >&2
      docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh coordinator \
        -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine" < "${ROLLBACK_DIR}/coordinator.tar.gz" >/dev/null 2>&1 || true
    fi

    # Roll back Storage Nodes
    for NODE in 1 2 3; do
      if [ -f "${ROLLBACK_DIR}/storage_node_${NODE}.tar.gz" ]; then
        echo "Rolling back Storage Node ${NODE} volume..." >&2
        docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
          -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/node-${NODE}" < "${ROLLBACK_DIR}/storage_node_${NODE}.tar.gz" >/dev/null 2>&1 || true
      fi
    done

    # Roll back API storage and SSH
    if [ -f "${ROLLBACK_DIR}/spine_ssh.tar.gz" ]; then
      echo "Rolling back API SSH volume..." >&2
      docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
        -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/ssh" < "${ROLLBACK_DIR}/spine_ssh.tar.gz" >/dev/null 2>&1 || true
    fi
    if [ -f "${ROLLBACK_DIR}/spine_storage.tar.gz" ]; then
      echo "Rolling back API storage volume..." >&2
      docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
        -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/storage" < "${ROLLBACK_DIR}/spine_storage.tar.gz" >/dev/null 2>&1 || true
    fi

    # Roll back PostgreSQL
    if [ -f "${ROLLBACK_DIR}/postgres_data.tar.gz" ]; then
      echo "Rolling back PostgreSQL volume..." >&2
      docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh postgres \
        -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/postgresql/data" < "${ROLLBACK_DIR}/postgres_data.tar.gz" >/dev/null 2>&1 || true
    fi

    echo "Rollback complete. Volumes restored to pre-restore state." >&2
    echo "Rollback artifacts retained for manual inspection at: ${ROLLBACK_DIR}" >&2
    echo "================================================================================" >&2
  fi
}
trap rollback_volumes EXIT ERR INT TERM

echo "=== Step 2: Preserving Rollback Archives & Restoring Clean Target Volumes ==="

# 1. Clean ephemeral staging in storage nodes (unquoted wildcards inside container shell)
for NODE in 1 2 3; do
  echo "Cleaning staging directories in storage node ${NODE}..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/.staged_restore_* /var/lib/spine/node-${NODE}/.old_restore_* 2>/dev/null || true"
done

# 2. Archive current volume states to ROLLBACK_DIR, completely empty volumes, then extract backup
echo "Preserving and restoring Coordinator volume..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar coordinator \
  -czf - -C /var/lib/spine . > "${ROLLBACK_DIR}/coordinator.tar.gz" || true
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh coordinator \
  -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* 2>/dev/null || true"
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh coordinator \
  -c "tar -xzf - -C /var/lib/spine" < "${BACKUP_DIR}/coordinator.tar.gz"

for NODE in 1 2 3; do
  echo "Preserving and restoring Storage Node ${NODE} volume..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar "storage-node-${NODE}" \
    -czf - -C "/var/lib/spine/node-${NODE}" . > "${ROLLBACK_DIR}/storage_node_${NODE}.tar.gz" || true
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* 2>/dev/null || true"
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "tar -xzf - -C /var/lib/spine/node-${NODE}" < "${BACKUP_DIR}/storage_node_${NODE}.tar.gz"
done

echo "Preserving and restoring API SSH identities..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/ssh . > "${ROLLBACK_DIR}/spine_ssh.tar.gz" || true
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* 2>/dev/null || true"
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "tar -xzf - -C /var/lib/spine/ssh" < "${BACKUP_DIR}/spine_ssh.tar.gz"

echo "Preserving and restoring API storage cache..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/storage . > "${ROLLBACK_DIR}/spine_storage.tar.gz" || true
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* 2>/dev/null || true"
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
  echo "HALTING DISASTER RECOVERY IMMEDIATELY. Rolling back volumes." >&2
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

echo "=== Step 5: Clean PostgreSQL Restoration & Verification ==="
# 1. Preserve existing PostgreSQL data into rollback dir, then make /var/lib/postgresql/data 100% EMPTY
echo "Preserving PostgreSQL volume and preparing completely empty PGDATA..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar postgres \
  -czf - -C /var/lib/postgresql/data . > "${ROLLBACK_DIR}/postgres_data.tar.gz" || true
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh postgres \
  -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* 2>/dev/null || true"

echo "Starting fresh PostgreSQL container..."
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

# 2. Terminate connections and drop empty compose-initialized spine_forge database
echo "Preparing database cluster for dump import..."
docker compose -f "${COMPOSE_FILE}" exec -T postgres psql -U "${POSTGRES_ADMIN_USER}" -d postgres -v ON_ERROR_STOP=1 -c "
  SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'spine_forge' AND pid <> pg_backend_pid();
  DROP DATABASE IF EXISTS spine_forge;
"

# 3. Import dump with ON_ERROR_STOP=1, adapting CREATE ROLE statements to handle existing bootstrap roles idempotently
echo "Importing PostgreSQL database from ${BACKUP_DIR}/postgres_dump.sql.gz (with ON_ERROR_STOP=1)..."
# Stream through sed: converts 'CREATE ROLE <name> ...;' into idempotent PL/pgSQL block
# Catching duplicate_object prevents collision with bootstrap roles while failing closed on any real error
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  gzip -dc "${BACKUP_DIR}/postgres_dump.sql.gz" | \
    sed -E 's/^CREATE ROLE ([^;]+);/DO \$\$ BEGIN CREATE ROLE \1; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;/' | \
    docker compose -f "${COMPOSE_FILE}" exec -T -e PGPASSWORD="${POSTGRES_ADMIN_PASSWORD}" postgres \
    psql -v ON_ERROR_STOP=1 -U "${POSTGRES_ADMIN_USER}" -d postgres
else
  gzip -dc "${BACKUP_DIR}/postgres_dump.sql.gz" | \
    sed -E 's/^CREATE ROLE ([^;]+);/DO \$\$ BEGIN CREATE ROLE \1; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;/' | \
    docker compose -f "${COMPOSE_FILE}" exec -T postgres \
    psql -v ON_ERROR_STOP=1 -U "${POSTGRES_ADMIN_USER}" -d postgres
fi

echo "Verifying restored PostgreSQL schema..."
TABLE_COUNT=$(docker compose -f "${COMPOSE_FILE}" exec -T postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d spine_forge -t -A -c \
  "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public';" 2>/dev/null || echo "0")

if [ "${TABLE_COUNT}" -eq 0 ]; then
  echo "FATAL: PostgreSQL restoration produced 0 tables in spine_forge public schema. Aborting." >&2
  exit 1
fi
echo "PostgreSQL restoration verified (${TABLE_COUNT} public tables present in spine_forge)."

echo "=== Step 6: Starting Core Infrastructure & Application Control Plane ==="
echo "Starting coordinator and storage nodes..."
docker compose -f "${COMPOSE_FILE}" up -d coordinator storage-node-1 storage-node-2 storage-node-3

echo "Awaiting core infrastructure health..."
sleep 5
docker compose -f "${COMPOSE_FILE}" ps

echo "Starting API, frontend, and ingress..."
docker compose -f "${COMPOSE_FILE}" up -d spine-api spine-frontend ingress

echo "=== Step 7: Readiness Health Check & Rollback Purge Gate ==="
sleep 3
READY_APP=0
for i in $(seq 1 30); do
  if curl -s -f http://127.0.0.1:3000/health/ready > /dev/null 2>&1; then
    READY_APP=1
    break
  fi
  sleep 2
done

if [ ${READY_APP} -ne 1 ]; then
  echo "FATAL: /health/ready returned non-200 after restoration. Halting without committing rollback purge." >&2
  curl -s http://127.0.0.1:3000/health/diagnostics || true
  exit 1
fi

RESTORE_SUCCESS=1
echo "Readiness gate passed. Purging pre-restore rollback artifacts..."
rm -rf "${ROLLBACK_DIR}"

echo "SUCCESS: Spine Forge cluster restored, verified, and fully ready at generation ${TARGET_GEN}."
