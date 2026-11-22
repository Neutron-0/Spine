#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Disaster Recovery Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail
umask 077
export MSYS_NO_PATHCONV=1
DOCKER_BIN="docker"
if command -v docker.exe >/dev/null 2>&1; then
  DOCKER_BIN="docker.exe"
fi

SPINE_API_BIN=""
if command -v spine-api >/dev/null 2>&1 && [ -s "$(command -v spine-api)" ]; then
  SPINE_API_BIN="spine-api"
elif command -v spine-api.exe >/dev/null 2>&1 && [ -s "$(command -v spine-api.exe)" ]; then
  SPINE_API_BIN="spine-api.exe"
elif [ -s "./target/release-candidate/spine-api.exe" ]; then
  SPINE_API_BIN="./target/release-candidate/spine-api.exe"
elif [ -s "./target/release-candidate/spine-api" ]; then
  SPINE_API_BIN="./target/release-candidate/spine-api"
elif [ -s "./github-alternative/backend/target/release/spine-api.exe" ]; then
  SPINE_API_BIN="./github-alternative/backend/target/release/spine-api.exe"
elif [ -s "./github-alternative/backend/target/release/spine-api" ]; then
  SPINE_API_BIN="./github-alternative/backend/target/release/spine-api"
elif [ -s "./github-alternative/backend/target/debug/spine-api.exe" ]; then
  SPINE_API_BIN="./github-alternative/backend/target/debug/spine-api.exe"
elif [ -s "./github-alternative/backend/target/debug/spine-api" ]; then
  SPINE_API_BIN="./github-alternative/backend/target/debug/spine-api"
fi

BACKUP_DIR=""
ENCRYPTED_FILE=""
TARGET_INPUT="${1:-}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
SPINE_API_URL="${SPINE_API_URL:-http://127.0.0.1:3000}"
COMPOSE_PROJECT_NAME="${COMPOSE_PROJECT_NAME:-}"
COMPOSE_ARGS=(-f "${COMPOSE_FILE}")
if [ -n "${COMPOSE_PROJECT_NAME}" ]; then
  COMPOSE_ARGS+=(-p "${COMPOSE_PROJECT_NAME}")
fi
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"
POSTGRES_DB="${POSTGRES_DB:-spine_forge}"
if [ -z "${POSTGRES_ADMIN_PASSWORD}" ] && [ -f .env ]; then
  POSTGRES_ADMIN_PASSWORD=$(grep -E "^POSTGRES_ADMIN_PASSWORD=" .env | head -n1 | cut -d'=' -f2- | tr -d '[:space:]"' || true)
fi
if [ "${POSTGRES_DB}" = "spine_forge" ] && [ -f .env ]; then
  env_db=$(grep -E "^POSTGRES_DB=" .env | head -n1 | cut -d'=' -f2- | tr -d '[:space:]"' || true)
  if [ -n "${env_db}" ]; then
    POSTGRES_DB="${env_db}"
  fi
fi
SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE:-}"
SPINE_BACKUP_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE:-}"
SPINE_S3_ENDPOINT_URL="${SPINE_S3_ENDPOINT_URL:-}"

RESOLVED_PASSPHRASE_FILE=""
CLEANUP_TEMP_PASSPHRASE=0

resolve_passphrase_file() {
  if [ -n "${SPINE_BACKUP_PASSPHRASE_FILE:-}" ] && [ -f "${SPINE_BACKUP_PASSPHRASE_FILE}" ]; then
    RESOLVED_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE}"
  elif [ -n "${SPINE_BACKUP_PASSPHRASE:-}" ]; then
    RESOLVED_PASSPHRASE_FILE=$(mktemp /tmp/spine_pass_XXXXXX 2>/dev/null || mktemp "${TMPDIR:-/tmp}/spine_pass_XXXXXX")
    chmod 600 "${RESOLVED_PASSPHRASE_FILE}"
    printf "%s" "${SPINE_BACKUP_PASSPHRASE}" > "${RESOLVED_PASSPHRASE_FILE}"
    CLEANUP_TEMP_PASSPHRASE=1
    unset SPINE_BACKUP_PASSPHRASE
  elif [ -f .env ] && grep -q -E "^SPINE_BACKUP_PASSPHRASE=" .env; then
    local env_pass
    env_pass=$(grep -E "^SPINE_BACKUP_PASSPHRASE=" .env | head -n1 | cut -d'=' -f2- | tr -d '[:space:]"' || true)
    if [ -n "${env_pass}" ]; then
      RESOLVED_PASSPHRASE_FILE=$(mktemp /tmp/spine_pass_XXXXXX 2>/dev/null || mktemp "${TMPDIR:-/tmp}/spine_pass_XXXXXX")
      chmod 600 "${RESOLVED_PASSPHRASE_FILE}"
      printf "%s" "${env_pass}" > "${RESOLVED_PASSPHRASE_FILE}"
      CLEANUP_TEMP_PASSPHRASE=1
    fi
  else
    echo "FATAL: SPINE_BACKUP_PASSPHRASE_FILE or SPINE_BACKUP_PASSPHRASE is required to authenticate and decrypt the backup archive." >&2
    exit 1
  fi

  if [ ! -s "${RESOLVED_PASSPHRASE_FILE}" ]; then
    echo "FATAL: Backup encryption passphrase file is empty or missing: ${RESOLVED_PASSPHRASE_FILE}" >&2
    exit 1
  fi
}

verify_supported_storage_topology() {
  echo "=== Verifying Supported Storage Topology ==="
  if [ ! -f "${COMPOSE_FILE}" ]; then
    echo "FATAL: Docker Compose configuration file not found: ${COMPOSE_FILE}" >&2
    exit 1
  fi

  local required_volumes=("postgres_data" "coordinator_data" "storage_1_data" "storage_2_data" "storage_3_data" "spine_storage" "spine_ssh")

  # 1. Parse compose file volume definitions for non-local drivers
  for vol in "${required_volumes[@]}"; do
    local custom_driver
    custom_driver=$(awk -v v="${vol}:" '
      $1 == v { in_vol=1; next }
      in_vol && /^[^ ]/ { in_vol=0 }
      in_vol && /^  [^ ]/ { in_vol=0 }
      in_vol && /driver:/ { print $2; exit }
    ' "${COMPOSE_FILE}" 2>/dev/null | tr -d '[:space:]"' | tr -d "'" || echo "")
    if [ -n "${custom_driver}" ] && [ "${custom_driver}" != "local" ]; then
      echo "FATAL: Volume '${vol}' in ${COMPOSE_FILE} is configured with unsupported driver '${custom_driver}'." >&2
      echo "Only driver 'local' is supported for production profile core-forge-v1." >&2
      exit 1
    fi
  done

  # 2. Check live Docker volumes if docker daemon is reachable
  if "${DOCKER_BIN}" info >/dev/null 2>&1; then
    local project_name="${COMPOSE_PROJECT_NAME:-}"
    if [ -z "${project_name}" ]; then
      project_name=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" config --format json 2>/dev/null | grep -o '"name": *"[^"]*"' | head -1 | cut -d'"' -f4 || echo "")
    fi
    if [ -z "${project_name}" ]; then
      project_name="spine"
    fi

    for vol in "${required_volumes[@]}"; do
      for candidate in "${project_name}_${vol}" "${vol}"; do
        local live_driver
        live_driver=$("${DOCKER_BIN}" volume inspect "${candidate}" --format '{{.Driver}}' 2>/dev/null || echo "")
        if [ -n "${live_driver}" ]; then
          if [ "${live_driver}" != "local" ]; then
            echo "FATAL: Docker volume '${candidate}' uses unsupported driver '${live_driver}'." >&2
            echo "Only driver 'local' is supported for production profile core-forge-v1." >&2
            exit 1
          fi
          break
        fi
      done
    done
  fi
  echo "Supported storage topology verified: Docker Compose with local volume driver."
}

if [ -z "${TARGET_INPUT}" ]; then
  echo "Usage: $0 <path-to-backup-dir-or-archive-or-s3-uri>" >&2
  echo "Example: $0 /mnt/backups/spine/20261117_120000Z_gen_42" >&2
  echo "Example: $0 /mnt/backups/spine/20261117_120000Z_gen_42/backup.spnebak" >&2
  echo "Example: $0 s3://spine-backups/20261117_120000Z_gen_42/backup.spnebak" >&2
  exit 1
fi

# State variables for cleanup and rollback handling
RESTORE_SUCCESS=0
VOLUMES_MODIFIED=0
to_host_mount_path() {
  local target="$1"
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "${target}"
  elif command -v wslpath >/dev/null 2>&1; then
    if [ -e "${target}" ]; then
      wslpath -w "${target}" 2>/dev/null || printf "%s" "${target}"
    else
      local dir
      dir=$(dirname "${target}")
      local base
      base=$(basename "${target}")
      local host_dir
      host_dir=$(wslpath -w "${dir}" 2>/dev/null || printf "%s" "${dir}")
      printf "%s\\%s" "${host_dir}" "${base}"
    fi
  else
    printf "%s" "${target}"
  fi
}

DECRYPT_DIR=""
ROLLBACK_DIR=""
REMOTE_RESTORE_DIR=""

encrypt_stream() {
  local host_pass_file
  host_pass_file=$(to_host_mount_path "${RESOLVED_PASSPHRASE_FILE}")
  if [ -n "${SPINE_API_BIN}" ]; then
    "${SPINE_API_BIN}" backup encrypt --passphrase-file "${host_pass_file}" --input - --output -
  else
    "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps \
      -v "${host_pass_file}:/run/secrets/backup_passphrase:ro" spine-api \
      backup encrypt --passphrase-file /run/secrets/backup_passphrase --input - --output -
  fi
}

decrypt_stream() {
  local host_pass_file
  host_pass_file=$(to_host_mount_path "${RESOLVED_PASSPHRASE_FILE}")
  if [ -n "${SPINE_API_BIN}" ]; then
    "${SPINE_API_BIN}" backup decrypt --passphrase-file "${host_pass_file}" --input - --output -
  else
    "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps \
      -v "${host_pass_file}:/run/secrets/backup_passphrase:ro" spine-api \
      backup decrypt --passphrase-file /run/secrets/backup_passphrase --input - --output -
  fi
}

restore_cleanup() {
  # 1. Clean up temporary remote download staging directory on every exit path (success, failure, signal)
  if [ -n "${REMOTE_RESTORE_DIR:-}" ] && [ -e "${REMOTE_RESTORE_DIR}" ]; then
    echo "Purging temporary remote download staging directory: ${REMOTE_RESTORE_DIR}..." >&2
    chmod -R 700 "${REMOTE_RESTORE_DIR}" 2>/dev/null || true
    if ! rm -rf "${REMOTE_RESTORE_DIR}"; then
      echo "ERROR: Failed to remove remote download staging directory: ${REMOTE_RESTORE_DIR}" >&2
    fi
  fi

  # 3. Always purge decrypted temporary payload directory on failure, abort, or signal
  if [ "${RESTORE_SUCCESS}" -eq 0 ]; then
    if [ -n "${DECRYPT_DIR:-}" ] && [ -e "${DECRYPT_DIR}" ]; then
      echo "Purging temporary decrypted restore directory: ${DECRYPT_DIR}..." >&2
      chmod -R 700 "${DECRYPT_DIR}" 2>/dev/null || true
      if ! rm -rf "${DECRYPT_DIR}"; then
        echo "ERROR: Failed to remove decrypted restore directory: ${DECRYPT_DIR}" >&2
      fi
    fi

    if [ "${VOLUMES_MODIFIED}" -eq 0 ]; then
      echo "NOTICE: Restore aborted before any volume was modified. Live data remains untouched." >&2
      if [ -n "${ROLLBACK_DIR:-}" ] && [ -e "${ROLLBACK_DIR}" ]; then
        chmod -R 700 "${ROLLBACK_DIR}" 2>/dev/null || true
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
      if ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" stop ingress spine-api storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend; then
        echo "ERROR: Failed to stop cluster services during rollback." >&2
        ROLLBACK_FAILED=1
      fi

      # 2. Mandatory quiescence verification (Fail-closed: MUST NOT extract if containers still running)
      RUNNING_ROLLBACK=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" ps --services --filter "status=running" 2>/dev/null || echo "CHECK_FAILED")
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
        if ! decrypt_stream < "${ROLLBACK_DIR}/coordinator.spnebak" | "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps --entrypoint sh coordinator \
          -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* && tar -xzf - -C /var/lib/spine"; then
          echo "ERROR: Failed to extract rollback archive for Coordinator!" >&2
          ROLLBACK_FAILED=1
        elif ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint test coordinator -f /var/lib/spine/.coordinator_fencing_generation; then
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
          if ! decrypt_stream < "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak" | "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps --entrypoint sh "storage-node-${NODE}" \
            -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* && tar -xzf - -C /var/lib/spine/node-${NODE}"; then
            echo "ERROR: Failed to extract rollback archive for Storage Node ${NODE}!" >&2
            ROLLBACK_FAILED=1
          elif ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint test "storage-node-${NODE}" -f "/var/lib/spine/node-${NODE}/.fencing_generation"; then
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
        if ! decrypt_stream < "${ROLLBACK_DIR}/spine_ssh.spnebak" | "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps --entrypoint sh spine-api \
          -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* && tar -xzf - -C /var/lib/spine/ssh"; then
          echo "ERROR: Failed to extract rollback archive for API SSH!" >&2
          ROLLBACK_FAILED=1
        elif ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint test spine-api -d /var/lib/spine/ssh; then
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
        if ! decrypt_stream < "${ROLLBACK_DIR}/spine_storage.spnebak" | "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps --entrypoint sh spine-api \
          -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* && tar -xzf - -C /var/lib/spine/storage"; then
          echo "ERROR: Failed to extract rollback archive for API storage!" >&2
          ROLLBACK_FAILED=1
        elif ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint test spine-api -d /var/lib/spine/storage; then
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
        if ! decrypt_stream < "${ROLLBACK_DIR}/postgres_data.spnebak" | "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps --entrypoint sh postgres \
          -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* && tar -xzf - -C /var/lib/postgresql/data"; then
          echo "ERROR: Failed to extract rollback archive for PostgreSQL!" >&2
          ROLLBACK_FAILED=1
        elif ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint test postgres -f /var/lib/postgresql/data/PG_VERSION; then
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

  # Final cleanup: purge temporary passphrase file if created
  if [ "${CLEANUP_TEMP_PASSPHRASE}" -eq 1 ] && [ -n "${RESOLVED_PASSPHRASE_FILE:-}" ] && [ -f "${RESOLVED_PASSPHRASE_FILE}" ]; then
    dd if=/dev/urandom of="${RESOLVED_PASSPHRASE_FILE}" bs=64 count=1 conv=notrunc 2>/dev/null || true
    rm -f "${RESOLVED_PASSPHRASE_FILE}"
  fi
}
handle_signal() {
  local sig="$1"
  echo "NOTICE: Signal ${sig} caught. Running restore cleanup and aborting..." >&2
  restore_cleanup
  if [ "${sig}" = "INT" ]; then
    exit 130
  else
    exit 143
  fi
}
# Install cleanup trap immediately to cover every exit, failure, and interrupt signal
trap restore_cleanup EXIT ERR
trap 'handle_signal INT' INT
trap 'handle_signal TERM' TERM

# Resolve passphrase and verify supported storage topology before proceeding
resolve_passphrase_file
verify_supported_storage_topology

# Handle remote S3 target download if specified
if [[ "${TARGET_INPUT}" =~ ^s3:// ]]; then
  echo "=== Detected Remote S3 Target: ${TARGET_INPUT} ==="
  AWS_BIN="aws"
  if ! command -v aws >/dev/null 2>&1 && command -v aws.exe >/dev/null 2>&1; then
    AWS_BIN="aws.exe"
  fi

  if ! command -v "${AWS_BIN}" >/dev/null 2>&1; then
    echo "FATAL: Restoring from S3 URI requires 'aws' CLI on host." >&2
    exit 1
  fi

  REMOTE_RESTORE_DIR=$(mktemp -d /tmp/spine_remote_restore_XXXXXX 2>/dev/null || mktemp -d "${TMPDIR:-/tmp}/spine_remote_restore_XXXXXX")
  chmod 700 "${REMOTE_RESTORE_DIR}"

  AWS_EXTRA_ARGS=()
  if [ -n "${SPINE_S3_ENDPOINT_URL}" ]; then
    AWS_EXTRA_ARGS+=(--endpoint-url "${SPINE_S3_ENDPOINT_URL}")
  fi
  if [ "${SPINE_S3_NO_SIGN_REQUEST:-0}" = "1" ] || [ "${AWS_NO_SIGN_REQUEST:-0}" = "1" ]; then
    AWS_EXTRA_ARGS+=(--no-sign-request)
  fi

  if [[ "${TARGET_INPUT}" =~ \.spnebak$ ]]; then
    REMOTE_ARCHIVE_URI="${TARGET_INPUT}"
    REMOTE_MANIFEST_URI="$(dirname "${TARGET_INPUT}")/SHA256SUMS"
  else
    REMOTE_ARCHIVE_URI="${TARGET_INPUT%/}/backup.spnebak"
    REMOTE_MANIFEST_URI="${TARGET_INPUT%/}/SHA256SUMS"
  fi

  host_remote_restore_dir=$(to_host_mount_path "${REMOTE_RESTORE_DIR}")
  echo "Downloading backup archive from ${REMOTE_ARCHIVE_URI}..."
  if ! "${AWS_BIN}" s3 cp "${REMOTE_ARCHIVE_URI}" "${host_remote_restore_dir}/backup.spnebak" "${AWS_EXTRA_ARGS[@]}"; then
    echo "FATAL: Failed to download remote backup archive from ${REMOTE_ARCHIVE_URI}" >&2
    exit 1
  fi

  echo "Downloading checksum manifest from ${REMOTE_MANIFEST_URI}..."
  if "${AWS_BIN}" s3 cp "${REMOTE_MANIFEST_URI}" "${host_remote_restore_dir}/SHA256SUMS" "${AWS_EXTRA_ARGS[@]}" 2>/dev/null; then
    echo "Verifying downloaded archive checksum against remote SHA256SUMS..."
    (cd "${REMOTE_RESTORE_DIR}" && sha256sum -c SHA256SUMS)
  else
    echo "NOTICE: Remote SHA256SUMS manifest not found on object store; proceeding with AEAD cryptographic integrity verification."
  fi

  TARGET_INPUT="${REMOTE_RESTORE_DIR}/backup.spnebak"
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

echo "=== Step 0a: Authenticating and Decrypting SPNEBAK v2 Backup Archive ==="
DECRYPT_DIR="${BACKUP_DIR}/.decrypted_restore_$(date +%s)"
mkdir -p "${DECRYPT_DIR}"
chmod 700 "${DECRYPT_DIR}"

DECRYPT_FAILED=0
mount_pass_file=$(to_host_mount_path "${RESOLVED_PASSPHRASE_FILE}")
mount_encrypted_file=$(to_host_mount_path "${ENCRYPTED_FILE}")
if [ -n "${SPINE_API_BIN}" ]; then
  if ! "${SPINE_API_BIN}" backup decrypt --passphrase-file "${mount_pass_file}" \
    --input "${mount_encrypted_file}" --output - | tar -xzf - -C "${DECRYPT_DIR}"; then
    DECRYPT_FAILED=1
  fi
else
  # Decrypt through spine-api container via read-only mounted secret file.
  # Credentials are never passed in process arguments or container environment variables.
  if ! "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm -i --no-deps \
    -v "${mount_pass_file}:/run/secrets/backup_passphrase:ro" spine-api \
    backup decrypt --passphrase-file /run/secrets/backup_passphrase --input - --output - < "${ENCRYPTED_FILE}" | tar -xzf - -C "${DECRYPT_DIR}"; then
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
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" stop ingress spine-api
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend

# Strictly verify no stateful containers are running
RUNNING=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" ps --services --filter "status=running")
if [ -n "${RUNNING}" ]; then
  echo "FATAL: The following services failed to stop and are still running:" >&2
  echo "${RUNNING}" >&2
  echo "Aborting restore to prevent data corruption." >&2
  exit 1
fi
echo "All cluster services confirmed stopped."

# Set up external rollback directory outside container volume paths and remote download dir
if [ -n "${REMOTE_RESTORE_DIR:-}" ]; then
  ROLLBACK_PARENT="/var/tmp"
  [ -w "${ROLLBACK_PARENT}" ] || ROLLBACK_PARENT="/tmp"
  ROLLBACK_DIR="${ROLLBACK_PARENT}/.pre_restore_rollback_$(date +%s)"
else
  ROLLBACK_DIR="${BACKUP_DIR}/.pre_restore_rollback_$(date +%s)"
fi
mkdir -p "${ROLLBACK_DIR}"
chmod 700 "${ROLLBACK_DIR}"

echo "=== Step 2a: Mandatory Pre-Restore Archiving & Integrity Gate (Volumes Untouched) ==="
# Capture pre-restore archives for EVERY participant without error suppression.
# All rollback artifacts are encrypted on the fly (SPNEBAK v2) to prevent persisting plaintext on disk.
# Any failure here aborts immediately before a single byte of live volume data is modified.

echo "Archiving cluster volumes concurrently (encrypted)..."
ARCH_PIDS=()
ARCH_NAMES=()

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint tar coordinator \
  -czf - -C /var/lib/spine . | encrypt_stream > "${ROLLBACK_DIR}/coordinator.spnebak" &
ARCH_PIDS+=($!)
ARCH_NAMES+=("coordinator")

for NODE in 1 2 3; do
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint tar "storage-node-${NODE}" \
    -czf - -C "/var/lib/spine/node-${NODE}" . | encrypt_stream > "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak" &
  ARCH_PIDS+=($!)
  ARCH_NAMES+=("storage_node_${NODE}")
done

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/ssh . | encrypt_stream > "${ROLLBACK_DIR}/spine_ssh.spnebak" &
ARCH_PIDS+=($!)
ARCH_NAMES+=("spine_ssh")

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint tar spine-api \
  -czf - -C /var/lib/spine/storage . | encrypt_stream > "${ROLLBACK_DIR}/spine_storage.spnebak" &
ARCH_PIDS+=($!)
ARCH_NAMES+=("spine_storage")

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint tar postgres \
  -czf - -C /var/lib/postgresql/data . | encrypt_stream > "${ROLLBACK_DIR}/postgres_data.spnebak" &
ARCH_PIDS+=($!)
ARCH_NAMES+=("postgres_data")

ARCH_FAIL=0
for i in "${!ARCH_PIDS[@]}"; do
  if ! wait "${ARCH_PIDS[$i]}"; then
    echo "ERROR: Archiving failed for ${ARCH_NAMES[$i]}." >&2
    ARCH_FAIL=1
  fi
done
if [ ${ARCH_FAIL} -ne 0 ]; then
  echo "FATAL: One or more pre-restore rollback archives failed to generate. Aborting restore." >&2
  exit 1
fi

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

# 1. Clean ephemeral staging in storage nodes
for NODE in 1 2 3; do
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/.staged_restore_* /var/lib/spine/node-${NODE}/.old_restore_* 2>/dev/null || true" &
done
wait

# 2. Empty target volumes and extract incoming backup cleanly in parallel
echo "Restoring cluster volumes concurrently in background..."
EXTRACT_PIDS=()
EXTRACT_NAMES=()

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh coordinator \
  -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine" < "${DECRYPT_DIR}/coordinator.tar.gz" &
EXTRACT_PIDS+=($!)
EXTRACT_NAMES+=("coordinator")

for NODE in 1 2 3; do
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
    -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/node-${NODE}" < "${DECRYPT_DIR}/storage_node_${NODE}.tar.gz" &
  EXTRACT_PIDS+=($!)
  EXTRACT_NAMES+=("storage-node-${NODE}")
done

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/ssh" < "${DECRYPT_DIR}/spine_ssh.tar.gz" &
EXTRACT_PIDS+=($!)
EXTRACT_NAMES+=("spine-api-ssh")

"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh spine-api \
  -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/storage" < "${DECRYPT_DIR}/spine_storage.tar.gz" &
EXTRACT_PIDS+=($!)
EXTRACT_NAMES+=("spine-api-storage")

EXTRACT_FAIL=0
for i in "${!EXTRACT_PIDS[@]}"; do
  if ! wait "${EXTRACT_PIDS[$i]}"; then
    echo "ERROR: Extraction failed for ${EXTRACT_NAMES[$i]}." >&2
    EXTRACT_FAIL=1
  else
    echo "Volume extraction succeeded for ${EXTRACT_NAMES[$i]} [OK]."
  fi
done
if [ ${EXTRACT_FAIL} -ne 0 ]; then
  echo "FATAL: One or more volume extractions failed. Aborting restore." >&2
  exit 1
fi

echo "=== Step 3: Automated Fail-Closed Fencing Generation Verification ==="
FAILED=0

COORD_GEN=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint cat coordinator \
  /var/lib/spine/.coordinator_fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")

if [ "${COORD_GEN}" != "${TARGET_GEN}" ]; then
  echo "ERROR: Coordinator generation (${COORD_GEN}) does not match target generation (${TARGET_GEN})."
  FAILED=1
else
  echo "Coordinator generation: ${COORD_GEN} [OK]"
fi

for NODE in 1 2 3; do
  NODE_GEN=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint cat "storage-node-${NODE}" \
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
  echo "   ${DOCKER_BIN} compose -f ${COMPOSE_FILE} run --rm coordinator \\" >&2
  echo "     repair-generation --expected-gen ${TARGET_GEN}" >&2
  echo "   (or flag syntax: --repair-generation --expected-gen ${TARGET_GEN})" >&2
  echo "4. Otherwise, abort and fall back to the previous verified backup." >&2
  echo "================================================================================" >&2
  exit 1
fi

echo "All participants match fencing generation ${TARGET_GEN}. Consensus verified."

echo "=== Step 4: Storage Node Volume Integrity Check ==="
VERIFY_PIDS=()
VERIFY_NODES=()
for NODE in 1 2 3; do
  echo "Verifying storage volume integrity on node ${NODE} in background..."
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm "storage-node-${NODE}" \
    --verify-integrity --data-dir "/var/lib/spine/node-${NODE}" &
  VERIFY_PIDS+=($!)
  VERIFY_NODES+=("${NODE}")
done

VERIFY_FAIL=0
for i in "${!VERIFY_PIDS[@]}"; do
  if ! wait "${VERIFY_PIDS[$i]}"; then
    echo "ERROR: Storage node ${VERIFY_NODES[$i]} failed volume integrity check." >&2
    VERIFY_FAIL=1
  else
    echo "Storage node ${VERIFY_NODES[$i]} integrity verified [OK]."
  fi
done
if [ ${VERIFY_FAIL} -ne 0 ]; then
  echo "FATAL: Storage node integrity checks failed. Aborting restore." >&2
  exit 1
fi

echo "=== Step 5: Clean PostgreSQL Restoration & Verification ==="
PG_EXEC_ARGS=()
if [ -n "${POSTGRES_ADMIN_PASSWORD}" ]; then
  PG_EXEC_ARGS+=(-e "PGPASSWORD=${POSTGRES_ADMIN_PASSWORD}")
fi

# Fast-path: Check if PGDATA is already initialized to skip 40-second redundant initdb
IS_INITIALIZED=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh postgres \
  -c "[ -s /var/lib/postgresql/data/PG_VERSION ] && echo 'YES' || echo 'NO'" 2>/dev/null || echo "NO")

if [ "${IS_INITIALIZED}" != "YES" ]; then
  echo "Preparing strictly empty PGDATA for fresh PostgreSQL initialization..."
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" run --rm --no-deps --entrypoint sh postgres \
    -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* 2>/dev/null || true"
  echo "Starting fresh PostgreSQL container..."
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" up -d postgres
else
  echo "PostgreSQL PGDATA is already initialized. Starting PostgreSQL container..."
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" up -d postgres
fi

echo "Awaiting PostgreSQL readiness..."
READY_PG=0
for _ in $(seq 1 75); do
  if "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T postgres pg_isready -U "${POSTGRES_ADMIN_USER}" >/dev/null 2>&1 && \
     "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres psql -U "${POSTGRES_ADMIN_USER}" -d postgres -c "SELECT 1;" >/dev/null 2>&1; then
    READY_PG=1
    break
  fi
  sleep 1
done

if [ ${READY_PG} -ne 1 ]; then
  echo "FATAL: PostgreSQL failed to become ready for database restoration." >&2
  exit 1
fi

echo "Preparing database cluster for high-throughput dump import..."
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres psql -U "${POSTGRES_ADMIN_USER}" -d postgres -v ON_ERROR_STOP=1 \
  -c "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = '${POSTGRES_DB}' AND pid <> pg_backend_pid();" \
  -c "DROP DATABASE IF EXISTS ${POSTGRES_DB};" \
  -c "DROP TABLE IF EXISTS _spine_active_database_selector CASCADE;"

# Temporarily optimize durability and WAL parameters for bulk restore speed
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres psql -U "${POSTGRES_ADMIN_USER}" -d postgres -v ON_ERROR_STOP=1 \
  -c "ALTER SYSTEM SET synchronous_commit = off;" \
  -c "ALTER SYSTEM SET checkpoint_timeout = '30min';" \
  -c "ALTER SYSTEM SET max_wal_size = '2GB';" \
  -c "SELECT pg_reload_conf();"

# Import dump with ON_ERROR_STOP=1, adapting CREATE ROLE statements to handle existing bootstrap roles idempotently
echo "Importing PostgreSQL database from ${DECRYPT_DIR}/postgres_dump.sql.gz (with ON_ERROR_STOP=1)..."
gzip -dc "${DECRYPT_DIR}/postgres_dump.sql.gz" | \
  sed -E -e 's/^CREATE ROLE ([^;]+);/DO \$\$ BEGIN CREATE ROLE \1; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;/' \
         -e 's/^CREATE TABLE _spine_active_database_selector/CREATE TABLE IF NOT EXISTS _spine_active_database_selector/' | \
  "${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -v ON_ERROR_STOP=1 -U "${POSTGRES_ADMIN_USER}" -d postgres

# Synchronously restore durability parameters and force checkpoint
echo "Restoring PostgreSQL transactional durability and synchronizing checkpoint..."
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres psql -U "${POSTGRES_ADMIN_USER}" -d postgres -v ON_ERROR_STOP=1 \
  -c "ALTER SYSTEM RESET synchronous_commit;" \
  -c "ALTER SYSTEM RESET checkpoint_timeout;" \
  -c "ALTER SYSTEM RESET max_wal_size;" \
  -c "SELECT pg_reload_conf();" \
  -c "CHECKPOINT;"

echo "Verifying restored PostgreSQL schema..."
TABLE_COUNT=$("${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -t -A -c \
  "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public';" 2>/dev/null || echo "0")

if [ "${TABLE_COUNT}" -eq 0 ]; then
  echo "FATAL: PostgreSQL restoration produced 0 tables in ${POSTGRES_DB} public schema. Aborting." >&2
  exit 1
fi
echo "PostgreSQL restoration verified (${TABLE_COUNT} public tables present in ${POSTGRES_DB})."

echo "Synchronizing PostgreSQL restore generation and barrier state to target generation ${TARGET_GEN}..."
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" exec -T "${PG_EXEC_ARGS[@]}" postgres \
  psql -U "${POSTGRES_ADMIN_USER}" -d "${POSTGRES_DB}" -v ON_ERROR_STOP=1 -c "
    INSERT INTO _restore_generation (id, generation_token, backup_id, restored_at_nanos, status)
    VALUES (1, '${TARGET_GEN}', 'oob-restore-${TARGET_GEN}', (EXTRACT(EPOCH FROM NOW()) * 1000000000)::bigint, 'COMMITTED')
    ON CONFLICT (id) DO UPDATE SET
      generation_token = EXCLUDED.generation_token,
      backup_id = EXCLUDED.backup_id,
      restored_at_nanos = EXCLUDED.restored_at_nanos,
      status = EXCLUDED.status;
    GRANT ALL PRIVILEGES ON TABLE _restore_generation TO ${POSTGRES_APP_USER:-spine_app};
    GRANT ALL PRIVILEGES ON TABLE _cluster_maintenance_barrier TO ${POSTGRES_APP_USER:-spine_app};
    UPDATE _cluster_maintenance_barrier
    SET is_active = FALSE,
        lease_expires_at_nanos = 0,
        owner_token = '',
        fencing_generation = ${TARGET_GEN}
    WHERE id = 1;
  "

echo "=== Step 6: Starting Core Infrastructure & Application Control Plane ==="
echo "Starting coordinator and storage nodes..."
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" up -d coordinator storage-node-1 storage-node-2 storage-node-3

echo "Awaiting core infrastructure health..."
sleep 5
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" ps

echo "Starting API, frontend, and ingress..."
"${DOCKER_BIN}" compose "${COMPOSE_ARGS[@]}" up -d spine-api spine-frontend ingress

echo "=== Step 7: Readiness Health Check & Rollback Purge Gate ==="
sleep 3
READY_APP=0
for _ in $(seq 1 30); do
  if curl -s -f "${SPINE_API_URL}/health/ready" > /dev/null 2>&1; then
    READY_APP=1
    break
  fi
  sleep 2
done

if [ ${READY_APP} -ne 1 ]; then
  echo "FATAL: /health/ready returned non-200 after restoration. Halting without committing rollback purge." >&2
  curl -s "${SPINE_API_URL}/health/diagnostics" || true
  exit 1
fi

RESTORE_SUCCESS=1
echo "Readiness gate passed. Purging pre-restore rollback artifacts, decrypted payload, and remote staging..."
rm -rf "${ROLLBACK_DIR}" "${DECRYPT_DIR}"
if [ -n "${REMOTE_RESTORE_DIR:-}" ] && [ -e "${REMOTE_RESTORE_DIR}" ]; then
  chmod -R 700 "${REMOTE_RESTORE_DIR}" 2>/dev/null || true
  rm -rf "${REMOTE_RESTORE_DIR}"
fi

echo "SUCCESS: Spine Forge cluster restored, verified, and fully ready at generation ${TARGET_GEN}."
