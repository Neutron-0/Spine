#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Disaster Recovery Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail
umask 077

BACKUP_DIR=""
ENCRYPTED_FILE=""
TARGET_INPUT="${1:-}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"
SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE:-}"
SPINE_BACKUP_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE:-}"

if [ -z "${SPINE_BACKUP_PASSPHRASE}" ] && [ -n "${SPINE_BACKUP_PASSPHRASE_FILE}" ] && [ -f "${SPINE_BACKUP_PASSPHRASE_FILE}" ]; then
  SPINE_BACKUP_PASSPHRASE=$(cat "${SPINE_BACKUP_PASSPHRASE_FILE}")
fi

if [ -z "${SPINE_BACKUP_PASSPHRASE}" ]; then
  echo "FATAL: SPINE_BACKUP_PASSPHRASE or SPINE_BACKUP_PASSPHRASE_FILE is required to authenticate and decrypt the backup archive." >&2
  exit 1
fi

if [ -z "${TARGET_INPUT}" ]; then
  echo "Usage: $0 <path-to-backup-dir-or-archive>" >&2
  echo "Example: $0 /mnt/backups/spine/20261117_120000Z_gen_42" >&2
  echo "Example: $0 /mnt/backups/spine/20261117_120000Z_gen_42/backup.spnebak" >&2
  exit 1
fi

if [ -f "${TARGET_INPUT}" ]; then
  ENCRYPTED_FILE="${TARGET_INPUT}"
  BACKUP_DIR="$(dirname "${TARGET_INPUT}")"
elif [ -d "${TARGET_INPUT}" ]; then
  BACKUP_DIR="${TARGET_INPUT}"
  ENCRYPTED_FILE="${BACKUP_DIR}/backup.spnebak"
else
  echo "FATAL: Target backup path does not exist: ${TARGET_INPUT}" >&2
  exit 1
fi

if [ ! -f "${ENCRYPTED_FILE}" ]; then
  echo "FATAL: Encrypted backup archive ${ENCRYPTED_FILE} does not exist." >&2
  echo "Under production profile 'core-forge-v1', restoring unencrypted plaintext archives is strictly forbidden." >&2
  exit 1
fi

# Verify format header magic: MUST be authenticated SPNEBAK v2 AEAD envelope
MAGIC_CHECK=$(dd if="${ENCRYPTED_FILE}" bs=1 count=8 2>/dev/null || true)
if [ "${MAGIC_CHECK}" != "SPNEBAK"$'\x02' ]; then
  echo "FATAL: Backup archive is unencrypted or invalid! Magic header does not match SPNEBAK v2." >&2
  echo "Under production profile 'core-forge-v1', restoring unencrypted plaintext archives is strictly forbidden." >&2
  exit 1
fi

# State variables for cleanup and rollback handling
RESTORE_SUCCESS=0
VOLUMES_MODIFIED=0
DECRYPT_DIR=""
ROLLBACK_DIR=""

encrypt_stream() {
  if command -v spine-api >/dev/null 2>&1; then
    SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" \
      spine-api backup encrypt --input - --output -
  else
    docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps \
      -e SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" spine-api \
      backup encrypt --input - --output -
  fi
}

decrypt_stream() {
  if command -v spine-api >/dev/null 2>&1; then
    SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" \
      spine-api backup decrypt --input - --output -
  else
    docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps \
      -e SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" spine-api \
      backup decrypt --input - --output -
  fi
}

restore_cleanup() {
  local exit_code=$?

  # 1. Always purge decrypted temporary payload directory on failure, abort, or signal
  if [ "${RESTORE_SUCCESS}" -eq 0 ]; then
    if [ -n "${DECRYPT_DIR:-}" ] && [ -d "${DECRYPT_DIR}" ]; then
      echo "Purging temporary decrypted restore directory: ${DECRYPT_DIR}..." >&2
      if ! rm -rf "${DECRYPT_DIR}"; then
        echo "ERROR: Failed to remove decrypted restore directory: ${DECRYPT_DIR}" >&2
      fi
    fi

    if [ "${VOLUMES_MODIFIED}" -eq 0 ]; then
      echo "NOTICE: Restore aborted before any volume was modified. Live data remains untouched." >&2
      if [ -n "${ROLLBACK_DIR:-}" ] && [ -d "${ROLLBACK_DIR}" ]; then
        rm -rf "${ROLLBACK_DIR}"
      fi
    elif [ -d "${ROLLBACK_DIR:-}" ]; then
      echo "================================================================================" >&2
      echo "CRITICAL: Restore failed after volume modification. EXECUTING AUTOMATIC ROLLBACK." >&2
      echo "Reverting all cluster volumes to pre-restore state..." >&2

      ROLLBACK_FAILED=0
      VERIFIED_COUNT=0

      # 1. Mandatory stop of any partially started services (NO || true)
      echo "Stopping cluster services before executing rollback extraction..." >&2
      if ! docker compose -f "${COMPOSE_FILE}" stop ingress spine-api storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend; then
        echo "ERROR: Failed to stop cluster services during rollback." >&2
        ROLLBACK_FAILED=1
      fi

      # 2. Mandatory quiescence verification (Fail-closed: MUST NOT extract if containers still running)
      RUNNING_ROLLBACK=$(docker compose -f "${COMPOSE_FILE}" ps --services --filter "status=running" 2>/dev/null || echo "CHECK_FAILED")
      if [ -n "${RUNNING_ROLLBACK}" ]; then
        echo "CRITICAL FATAL: Cannot execute rollback extraction because containers are still running:" >&2
        echo "${RUNNING_ROLLBACK}" >&2
        echo "Refusing to extract rollback archives into active volumes to prevent live data tearing." >&2
        echo "Cluster services remain strictly stopped." >&2
        ROLLBACK_FAILED=1
      fi

      # If stopping failed or containers refused to stop, ABORT rollback extraction immediately!
      if [ "${ROLLBACK_FAILED}" -ne 0 ]; then
        echo "================================================================================" >&2
        echo "CRITICAL FATAL: Automatic rollback extraction halted because services refused to stop." >&2
        echo "Cluster services remain STRICTLY STOPPED to prevent further data corruption." >&2
        echo "Rollback artifacts preserved in encrypted format (SPNEBAK v2) at: ${ROLLBACK_DIR}" >&2
        echo "OPERATOR ACTION REQUIRED: Terminate hung containers and inspect volumes manually." >&2
        echo "================================================================================" >&2
        exit 2
      fi

      # 3. Roll back Coordinator from encrypted archive
      if [ -f "${ROLLBACK_DIR}/coordinator.spnebak" ]; then
        echo "Rolling back Coordinator volume from encrypted archive..." >&2
        if ! decrypt_stream < "${ROLLBACK_DIR}/coordinator.spnebak" | docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps --entrypoint sh coordinator \
          -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* && tar -xzf - -C /var/lib/spine"; then
          echo "ERROR: Failed to extract rollback archive for Coordinator!" >&2
          ROLLBACK_FAILED=1
        elif ! docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint test coordinator -f /var/lib/spine/.coordinator_fencing_generation; then
          echo "ERROR: Coordinator volume verification failed after rollback (.coordinator_fencing_generation missing)!" >&2
          ROLLBACK_FAILED=1
        else
          echo "Coordinator volume rolled back and verified [OK]" >&2
          VERIFIED_COUNT=$((VERIFIED_COUNT + 1))
        fi
      else
        echo "ERROR: Missing rollback archive for Coordinator (${ROLLBACK_DIR}/coordinator.spnebak)!" >&2
        ROLLBACK_FAILED=1
      fi

      # 4. Roll back Storage Nodes (1, 2, 3) from encrypted archives
      for NODE in 1 2 3; do
        if [ -f "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak" ]; then
          echo "Rolling back Storage Node ${NODE} volume from encrypted archive..." >&2
          if ! decrypt_stream < "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak" | docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps --entrypoint sh "storage-node-${NODE}" \
            -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* && tar -xzf - -C /var/lib/spine/node-${NODE}"; then
            echo "ERROR: Failed to extract rollback archive for Storage Node ${NODE}!" >&2
            ROLLBACK_FAILED=1
          elif ! docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint test "storage-node-${NODE}" -f "/var/lib/spine/node-${NODE}/.fencing_generation"; then
            echo "ERROR: Storage Node ${NODE} volume verification failed after rollback (.fencing_generation missing)!" >&2
            ROLLBACK_FAILED=1
          else
            echo "Storage Node ${NODE} volume rolled back and verified [OK]" >&2
            VERIFIED_COUNT=$((VERIFIED_COUNT + 1))
          fi
        else
          echo "ERROR: Missing rollback archive for Storage Node ${NODE} (${ROLLBACK_DIR}/storage_node_${NODE}.spnebak)!" >&2
          ROLLBACK_FAILED=1
        fi
      done

      # 5. Roll back API SSH from encrypted archive
      if [ -f "${ROLLBACK_DIR}/spine_ssh.spnebak" ]; then
        echo "Rolling back API SSH volume from encrypted archive..." >&2
        if ! decrypt_stream < "${ROLLBACK_DIR}/spine_ssh.spnebak" | docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps --entrypoint sh spine-api \
          -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* && tar -xzf - -C /var/lib/spine/ssh"; then
          echo "ERROR: Failed to extract rollback archive for API SSH!" >&2
          ROLLBACK_FAILED=1
        elif ! docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint test spine-api -d /var/lib/spine/ssh; then
          echo "ERROR: API SSH volume verification failed after rollback (/var/lib/spine/ssh missing)!" >&2
          ROLLBACK_FAILED=1
        else
          echo "API SSH volume rolled back and verified [OK]" >&2
          VERIFIED_COUNT=$((VERIFIED_COUNT + 1))
        fi
      else
        echo "ERROR: Missing rollback archive for API SSH (${ROLLBACK_DIR}/spine_ssh.spnebak)!" >&2
        ROLLBACK_FAILED=1
      fi

      # 6. Roll back API storage from encrypted archive
      if [ -f "${ROLLBACK_DIR}/spine_storage.spnebak" ]; then
        echo "Rolling back API storage volume from encrypted archive..." >&2
        if ! decrypt_stream < "${ROLLBACK_DIR}/spine_storage.spnebak" | docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps --entrypoint sh spine-api \
          -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* && tar -xzf - -C /var/lib/spine/storage"; then
          echo "ERROR: Failed to extract rollback archive for API storage!" >&2
          ROLLBACK_FAILED=1
        elif ! docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint test spine-api -d /var/lib/spine/storage; then
          echo "ERROR: API storage volume verification failed after rollback (/var/lib/spine/storage missing)!" >&2
          ROLLBACK_FAILED=1
        else
          echo "API storage volume rolled back and verified [OK]" >&2
          VERIFIED_COUNT=$((VERIFIED_COUNT + 1))
        fi
      else
        echo "ERROR: Missing rollback archive for API storage (${ROLLBACK_DIR}/spine_storage.spnebak)!" >&2
        ROLLBACK_FAILED=1
      fi

      # 7. Roll back PostgreSQL from encrypted archive
      if [ -f "${ROLLBACK_DIR}/postgres_data.spnebak" ]; then
        echo "Rolling back PostgreSQL volume from encrypted archive..." >&2
        if ! decrypt_stream < "${ROLLBACK_DIR}/postgres_data.spnebak" | docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps --entrypoint sh postgres \
          -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* && tar -xzf - -C /var/lib/postgresql/data"; then
          echo "ERROR: Failed to extract rollback archive for PostgreSQL!" >&2
          ROLLBACK_FAILED=1
        elif ! docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint test postgres -f /var/lib/postgresql/data/PG_VERSION; then
          echo "ERROR: PostgreSQL volume verification failed after rollback (PG_VERSION missing)!" >&2
          ROLLBACK_FAILED=1
        else
          echo "PostgreSQL volume rolled back and verified [OK]" >&2
          VERIFIED_COUNT=$((VERIFIED_COUNT + 1))
        fi
      else
        echo "ERROR: Missing rollback archive for PostgreSQL (${ROLLBACK_DIR}/postgres_data.spnebak)!" >&2
        ROLLBACK_FAILED=1
      fi

      # 8. Global Rollback Verification Gate
      if [ "${ROLLBACK_FAILED}" -ne 0 ] || [ "${VERIFIED_COUNT}" -ne 7 ]; then
        echo "================================================================================" >&2
        echo "CRITICAL FATAL: Automatic rollback FAILED. Only ${VERIFIED_COUNT}/7 volumes verified." >&2
        echo "Cluster services remain STRICTLY STOPPED to prevent further data corruption." >&2
        echo "Rollback artifacts preserved in encrypted format (SPNEBAK v2) at: ${ROLLBACK_DIR}" >&2
        echo "OPERATOR ACTION REQUIRED: Do NOT start services. Inspect failed volumes manually." >&2
        echo "================================================================================" >&2
        exit 2
      fi

      echo "Rollback complete and all 7/7 volumes verified. Cluster reverted to pre-restore state." >&2
      echo "Cluster remains stopped to allow operator investigation." >&2
      echo "Rollback artifacts preserved in encrypted format (SPNEBAK v2) at: ${ROLLBACK_DIR}" >&2
      echo "OPERATOR ACTION: Once investigation is complete, remove encrypted rollback artifacts with: rm -rf ${ROLLBACK_DIR}" >&2
      echo "================================================================================" >&2
    fi
  fi
}
# Install cleanup trap immediately to cover every failure and interrupt path
trap restore_cleanup EXIT ERR INT TERM

echo "=== Step 0a: Authenticating and Decrypting SPNEBAK v2 Backup Archive ==="
DECRYPT_DIR="${BACKUP_DIR}/.decrypted_restore_$(date +%s)"
mkdir -p "${DECRYPT_DIR}"
chmod 700 "${DECRYPT_DIR}"

DECRYPT_FAILED=0
if command -v spine-api >/dev/null 2>&1; then
  if ! SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" \
    spine-api backup decrypt --input "${ENCRYPTED_FILE}" --output - | tar -xzf - -C "${DECRYPT_DIR}"; then
    DECRYPT_FAILED=1
  fi
else
  if ! docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps \
    -e SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" spine-api \
    backup decrypt --input - --output - < "${ENCRYPTED_FILE}" | tar -xzf - -C "${DECRYPT_DIR}"; then
    DECRYPT_FAILED=1
  fi
fi

if [ ${DECRYPT_FAILED} -ne 0 ]; then
  echo "================================================================================" >&2
  echo "CRITICAL FATAL: Backup authentication or decryption failed!" >&2
  echo "The backup archive is corrupted, tampered, or the encryption passphrase is invalid." >&2
  echo "ABORTING RESTORE IMMEDIATELY. Zero cluster services have been touched." >&2
  echo "================================================================================" >&2
  exit 1
fi

echo "Backup authentication verified [OK]. All AES-256-GCM chunk MACs and SHA-256 manifest valid."

if [ ! -f "${DECRYPT_DIR}/FENCING_GENERATION" ] || [ ! -f "${DECRYPT_DIR}/SHA256SUMS" ]; then
  echo "FATAL: Invalid decrypted backup contents. FENCING_GENERATION or SHA256SUMS missing." >&2
  exit 1
fi

TARGET_GEN=$(cat "${DECRYPT_DIR}/FENCING_GENERATION" | tr -d '[:space:]')
echo "Target recovery generation: ${TARGET_GEN}"

echo "=== Step 0b: Verifying Restored Payload SHA-256 Checksum Manifest ==="
(cd "${DECRYPT_DIR}" && sha256sum -c SHA256SUMS)

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
chmod 700 "${ROLLBACK_DIR}"

echo "=== Step 2a: Mandatory Pre-Restore Archiving & Integrity Gate (Volumes Untouched) ==="
# Capture pre-restore archives for EVERY participant without error suppression.
# All rollback artifacts are encrypted on the fly (SPNEBAK v2) to prevent persisting plaintext on disk.
# Any failure here aborts immediately before a single byte of live volume data is modified.

echo "Archiving Coordinator volume (encrypted)..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar coordinator \
  -czf - -C /var/lib/spine . | encrypt_stream > "${ROLLBACK_DIR}/coordinator.spnebak"

for NODE in 1 2 3; do
  echo "Archiving Storage Node ${NODE} volume (encrypted)..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar "storage-node-${NODE}" \
    -czf - -C "/var/lib/spine/node-${NODE}" . | encrypt_stream > "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak"
done

echo "Archiving API SSH identities (encrypted)..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/ssh . | encrypt_stream > "${ROLLBACK_DIR}/spine_ssh.spnebak"

echo "Archiving API storage cache (encrypted)..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/storage . | encrypt_stream > "${ROLLBACK_DIR}/spine_storage.spnebak"

echo "Archiving PostgreSQL volume (encrypted)..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint tar postgres \
  -czf - -C /var/lib/postgresql/data . | encrypt_stream > "${ROLLBACK_DIR}/postgres_data.spnebak"

# Restrictive permissions on rollback directory and encrypted archives
chmod 600 "${ROLLBACK_DIR}"/*
chmod 700 "${ROLLBACK_DIR}"

# Rigorous verification gate: all 7 rollback archives must exist, be non-empty, and pass AEAD authentication and tar integrity check
for ARCHIVE in coordinator storage_node_1 storage_node_2 storage_node_3 spine_ssh spine_storage postgres_data; do
  FILE="${ROLLBACK_DIR}/${ARCHIVE}.spnebak"
  if [ ! -s "${FILE}" ]; then
    echo "FATAL: Pre-restore rollback archive ${FILE} is missing or zero-byte empty." >&2
    echo "ABORTING RESTORE IMMEDIATELY. Zero volumes have been modified." >&2
    exit 1
  fi
  MAGIC_CHECK=$(dd if="${FILE}" bs=1 count=8 2>/dev/null || true)
  if [ "${MAGIC_CHECK}" != "SPNEBAK"$'\x02' ]; then
    echo "FATAL: Pre-restore rollback archive ${FILE} is not a valid SPNEBAK v2 encrypted envelope." >&2
    echo "ABORTING RESTORE IMMEDIATELY. Zero volumes have been modified." >&2
    exit 1
  fi
  if ! decrypt_stream < "${FILE}" | tar -tzf - >/dev/null 2>&1; then
    echo "FATAL: Pre-restore rollback archive ${FILE} failed AEAD authentication or tar integrity check." >&2
    echo "ABORTING RESTORE IMMEDIATELY. Zero volumes have been modified." >&2
    exit 1
  fi
  echo "Verified encrypted rollback archive: ${ARCHIVE}.spnebak [OK]"
done

echo "All 7 pre-restore rollback archives verified valid and encrypted. Safe to proceed with volume modification."
VOLUMES_MODIFIED=1

echo "=== Step 2b: Cleaning Target Volumes & Extracting Incoming Backup ==="

# 1. Clean ephemeral staging in storage nodes (unquoted wildcards inside container shell)
for NODE in 1 2 3; do
  echo "Cleaning staging directories in storage node ${NODE}..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/.staged_restore_* /var/lib/spine/node-${NODE}/.old_restore_* 2>/dev/null || true"
done

# 2. Empty target volumes and extract incoming backup cleanly
echo "Restoring Coordinator volume..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh coordinator \
  -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine" < "${DECRYPT_DIR}/coordinator.tar.gz"

for NODE in 1 2 3; do
  echo "Restoring Storage Node ${NODE} volume..."
  docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/node-${NODE}" < "${DECRYPT_DIR}/storage_node_${NODE}.tar.gz"
done

echo "Restoring API SSH identities..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/ssh" < "${DECRYPT_DIR}/spine_ssh.tar.gz"

echo "Restoring API storage cache..."
docker compose -f "${COMPOSE_FILE}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/storage" < "${DECRYPT_DIR}/spine_storage.tar.gz"

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
# 1. Strictly empty /var/lib/postgresql/data for PostgreSQL initdb
echo "Preparing strictly empty PGDATA for fresh PostgreSQL initialization..."
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
echo "Importing PostgreSQL database from ${DECRYPT_DIR}/postgres_dump.sql.gz (with ON_ERROR_STOP=1)..."
# Stream through sed: converts 'CREATE ROLE <name> ...;' into idempotent PL/pgSQL block
# Catching duplicate_object prevents collision with bootstrap roles while failing closed on any real error
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  gzip -dc "${DECRYPT_DIR}/postgres_dump.sql.gz" | \
    sed -E 's/^CREATE ROLE ([^;]+);/DO \$\$ BEGIN CREATE ROLE \1; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;/' | \
    docker compose -f "${COMPOSE_FILE}" exec -T -e PGPASSWORD="${POSTGRES_ADMIN_PASSWORD}" postgres \
    psql -v ON_ERROR_STOP=1 -U "${POSTGRES_ADMIN_USER}" -d postgres
else
  gzip -dc "${DECRYPT_DIR}/postgres_dump.sql.gz" | \
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
echo "Readiness gate passed. Purging pre-restore rollback artifacts and decrypted payload..."
rm -rf "${ROLLBACK_DIR}" "${DECRYPT_DIR}"

echo "SUCCESS: Spine Forge cluster restored, verified, and fully ready at generation ${TARGET_GEN}."
