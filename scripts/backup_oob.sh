#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Out-of-Band Multi-Volume Backup Script
# Canonical Certified Procedure for Clusters > 15 MiB
# ==============================================================================
set -euo pipefail
umask 077
export MSYS_NO_PATHCONV=1

SPINE_API_URL="${SPINE_API_URL:-http://127.0.0.1:3000}"
SPINE_ADMIN_TOKEN="${SPINE_ADMIN_TOKEN:-}"
SPINE_ADMIN_TOKEN_FILE="${SPINE_ADMIN_TOKEN_FILE:-}"
BACKUP_ROOT="${BACKUP_ROOT:-/mnt/backups/spine}"
COMPOSE_FILE="${COMPOSE_FILE:-docker-compose.prod.yml}"
POSTGRES_ADMIN_USER="${POSTGRES_ADMIN_USER:-spine_admin}"
POSTGRES_ADMIN_PASSWORD="${POSTGRES_ADMIN_PASSWORD:-}"
SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE:-}"
SPINE_BACKUP_PASSPHRASE_FILE="${SPINE_BACKUP_PASSPHRASE_FILE:-}"
SPINE_DEPLOYMENT_PROFILE="${SPINE_DEPLOYMENT_PROFILE:-core-forge-v1}"
SPINE_BACKUP_REMOTE_S3_URI="${SPINE_BACKUP_REMOTE_S3_URI:-}"
SPINE_BACKUP_REQUIRE_REMOTE="${SPINE_BACKUP_REQUIRE_REMOTE:-${SPINE_BACKUP_REQUIRE_OFF_SITE:-1}}"
SPINE_BACKUP_OBJECT_LOCK_MODE="${SPINE_BACKUP_OBJECT_LOCK_MODE:-COMPLIANCE}"
SPINE_BACKUP_RETENTION_DAYS="${SPINE_BACKUP_RETENTION_DAYS:-30}"
SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE="${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE:-0}"
SPINE_S3_ENDPOINT_URL="${SPINE_S3_ENDPOINT_URL:-}"

if [ -z "${SPINE_ADMIN_TOKEN}" ] && [ -n "${SPINE_ADMIN_TOKEN_FILE}" ] && [ -f "${SPINE_ADMIN_TOKEN_FILE}" ]; then
  SPINE_ADMIN_TOKEN="$(tr -d '\r\n' < "${SPINE_ADMIN_TOKEN_FILE}")"
fi

if [ -z "${SPINE_ADMIN_TOKEN}" ]; then
  echo "FATAL: SPINE_ADMIN_TOKEN or SPINE_ADMIN_TOKEN_FILE must be set to acquire the cluster maintenance barrier." >&2
  exit 1
fi

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
  else
    echo "FATAL: Authenticated encryption is mandatory for production backups." >&2
    echo "Please provide SPINE_BACKUP_PASSPHRASE_FILE or set SPINE_BACKUP_PASSPHRASE." >&2
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
  if command -v docker >/dev/null 2>&1; then
    local project_name
    project_name=$(docker compose -f "${COMPOSE_FILE}" config --format json 2>/dev/null | grep -o '"name": *"[^"]*"' | head -1 | cut -d'"' -f4 || echo "")
    if [ -z "${project_name}" ]; then
      project_name="spine"
    fi

    for vol in "${required_volumes[@]}"; do
      for candidate in "${project_name}_${vol}" "${vol}"; do
        local live_driver
        live_driver=$(docker volume inspect "${candidate}" --format '{{.Driver}}' 2>/dev/null || echo "")
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

compute_retain_until_date() {
  local days="$1"
  local d=""
  d=$(date -u -d "+${days} days" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  if [ -z "${d}" ]; then
    d=$(date -u -v "+${days}d" +"%Y-%m-%dT%H:%M:%SZ" 2>/dev/null || true)
  fi
  printf "%s" "${d}"
}

iso_to_epoch() {
  local iso_str="${1:-}"
  if [ -z "${iso_str}" ]; then
    return 0
  fi
  local clean_str
  clean_str=$(echo "${iso_str}" | sed -E 's/\.[0-9]+Z$/Z/')
  if ! [[ "${clean_str}" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2} ]]; then
    return 0
  fi
  local epoch=""
  epoch=$(date -u -d "${clean_str}" +%s 2>/dev/null || true)
  if [ -z "${epoch}" ]; then
    epoch=$(date -u -d "${iso_str}" +%s 2>/dev/null || true)
  fi
  if [ -z "${epoch}" ]; then
    epoch=$(date -j -u -f "%Y-%m-%dT%H:%M:%SZ" "${clean_str}" +%s 2>/dev/null || true)
  fi
  if [ -z "${epoch}" ]; then
    epoch=$(python3 -c "import datetime, sys; s = sys.argv[1].replace('Z', '+00:00'); print(int(datetime.datetime.fromisoformat(s).timestamp()))" "${iso_str}" 2>/dev/null || true)
  fi
  printf "%s" "${epoch}"
}

normalize_deployment_profile() {
  case "${SPINE_DEPLOYMENT_PROFILE}" in
    core-forge-v1|core-forge|production|prod)
      echo "core-forge-v1"
      ;;
    development|dev)
      echo "development"
      ;;
    test)
      echo "test"
      ;;
    *)
      echo "invalid"
      ;;
  esac
}

verify_off_site_replication_policy() {
  echo "=== Verifying Off-Site Replication Policy ==="
  local profile
  profile=$(normalize_deployment_profile)
  if [ "${profile}" = "invalid" ]; then
    echo "================================================================================" >&2
    echo "FATAL: Invalid SPINE_DEPLOYMENT_PROFILE='${SPINE_DEPLOYMENT_PROFILE}'." >&2
    echo "Supported deployment profiles: 'core-forge-v1' (production), 'development', 'test'." >&2
    echo "================================================================================" >&2
    exit 1
  fi
  echo "Deployment profile: ${SPINE_DEPLOYMENT_PROFILE} (normalized: ${profile})"

  case "${SPINE_BACKUP_REQUIRE_REMOTE}" in
    1|true)
      if [ -z "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
        echo "================================================================================" >&2
        echo "FATAL: Off-site replication policy is ENFORCED (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE})," >&2
        echo "but SPINE_BACKUP_REMOTE_S3_URI is not set." >&2
        echo "Under production profile 'core-forge-v1', backups must be replicated to an" >&2
        echo "off-host object store with Object Lock retention to survive total host destruction." >&2
        echo "To explicitly allow local-only backups for testing or dev, set SPINE_BACKUP_REQUIRE_REMOTE=0." >&2
        echo "================================================================================" >&2
        exit 1
      fi
      if ! command -v aws >/dev/null 2>&1; then
        echo "FATAL: Off-site replication is required (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE}), but 'aws' CLI is not found on host." >&2
        exit 1
      fi
      echo "Off-site replication policy verified [ENFORCED] -> Target: ${SPINE_BACKUP_REMOTE_S3_URI}"
      ;;
    0|false)
      echo "Off-site replication policy: INTENTIONALLY LOCAL-ONLY (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE})."
      echo "OPERATIONAL NOTICE: Local-only backups do not protect against total host loss."
      ;;
    *)
      echo "================================================================================" >&2
      echo "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value: '${SPINE_BACKUP_REQUIRE_REMOTE}'" >&2
      echo "Strict policy validation: SPINE_BACKUP_REQUIRE_REMOTE must be strictly '1' (or 'true')" >&2
      echo "to enforce off-site replication, or '0' (or 'false') to explicitly permit local-only backups." >&2
      echo "All other values (e.g. typos like 'yes', 'no', '2') are rejected fail-closed." >&2
      echo "================================================================================" >&2
      exit 1
      ;;
  esac

  case "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" in
    0|1|false|true)
      ;;
    *)
      echo "================================================================================" >&2
      echo "FATAL: Invalid SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE value: '${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}'" >&2
      echo "Must be strictly '0', '1', 'false', or 'true'." >&2
      echo "================================================================================" >&2
      exit 1
      ;;
  esac

  # Validate remote retention configuration whenever S3 replication is configured
  if [ -n "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
    # Production profile check: core-forge-v1 strictly forbids unlocked remote replication
    if [ "${profile}" = "core-forge-v1" ]; then
      if [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" = "1" ] || [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" = "true" ]; then
        echo "================================================================================" >&2
        echo "FATAL: S3 Object Lock retention bypass (SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1) is" >&2
        echo "strictly FORBIDDEN under production profile 'core-forge-v1'." >&2
        echo "Production backups must have immutable Object Lock retention to guarantee resilience" >&2
        echo "against ransomware, compromised credentials, and operator error." >&2
        echo "The unlocked bypass is only available under non-production profiles ('development' or 'test')." >&2
        echo "================================================================================" >&2
        exit 1
      fi

      if [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "GOVERNANCE" ]; then
        echo "================================================================================" >&2
        echo "FATAL: S3 Object Lock mode 'GOVERNANCE' is rejected under production profile 'core-forge-v1'." >&2
        echo "Under production profile 'core-forge-v1', backups require non-bypassable WORM immutability" >&2
        echo "('COMPLIANCE') to protect against host compromise and privileged ransomware." >&2
        echo "'GOVERNANCE' mode is bypassable by principals with s3:BypassGovernanceRetention permissions" >&2
        echo "and is strictly reserved for non-production profiles ('development' or 'test')." >&2
        echo "================================================================================" >&2
        exit 1
      fi
    fi

    case "${SPINE_BACKUP_OBJECT_LOCK_MODE}" in
      COMPLIANCE)
        if ! [[ "${SPINE_BACKUP_RETENTION_DAYS}" =~ ^[1-9][0-9]*$ ]]; then
          echo "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS='${SPINE_BACKUP_RETENTION_DAYS}'. Must be a positive integer." >&2
          exit 1
        fi
        local test_date
        test_date=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
        if [ -z "${test_date}" ]; then
          echo "FATAL: Cannot compute retain-until date for S3 Object Lock retention (+${SPINE_BACKUP_RETENTION_DAYS} days)." >&2
          echo "Neither GNU date nor BSD date produced a valid UTC ISO 8601 timestamp." >&2
          echo "Date calculation is required to enforce immutable retention. Failing closed before snapshot capture." >&2
          exit 1
        fi
        echo "S3 Object Lock retention policy verified [PRODUCTION COMPLIANCE]: mode=COMPLIANCE, days=${SPINE_BACKUP_RETENTION_DAYS} (sample retain-until: ${test_date})."
        ;;
      GOVERNANCE)
        if ! [[ "${SPINE_BACKUP_RETENTION_DAYS}" =~ ^[1-9][0-9]*$ ]]; then
          echo "FATAL: Invalid SPINE_BACKUP_RETENTION_DAYS='${SPINE_BACKUP_RETENTION_DAYS}'. Must be a positive integer." >&2
          exit 1
        fi
        local test_date
        test_date=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
        if [ -z "${test_date}" ]; then
          echo "FATAL: Cannot compute retain-until date for S3 Object Lock retention (+${SPINE_BACKUP_RETENTION_DAYS} days)." >&2
          echo "Neither GNU date nor BSD date produced a valid UTC ISO 8601 timestamp." >&2
          echo "Date calculation is required to enforce immutable retention. Failing closed before snapshot capture." >&2
          exit 1
        fi
        echo "S3 Object Lock retention policy verified [NON-PRODUCTION GOVERNANCE]: mode=GOVERNANCE, days=${SPINE_BACKUP_RETENTION_DAYS} (sample retain-until: ${test_date})."
        ;;
      ""|NONE)
        if [ "${profile}" = "core-forge-v1" ]; then
          echo "================================================================================" >&2
          echo "FATAL: S3 Object Lock retention is MANDATORY under production profile 'core-forge-v1'" >&2
          echo "when off-site replication is configured. Omitting Object Lock is strictly forbidden." >&2
          echo "================================================================================" >&2
          exit 1
        fi

        if [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "1" ] || [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "true" ]; then
          if [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" != "1" ] && [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" != "true" ]; then
            echo "================================================================================" >&2
            echo "FATAL: S3 Object Lock retention is mandatory when off-site replication is enforced" >&2
            echo "(SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE}), but SPINE_BACKUP_OBJECT_LOCK_MODE is disabled ('${SPINE_BACKUP_OBJECT_LOCK_MODE}')." >&2
            echo "To explicitly allow un-locked remote replication for non-production testing, set" >&2
            echo "SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1." >&2
            echo "================================================================================" >&2
            exit 1
          fi
          echo "================================================================================"
          echo "NON-PRODUCTION NOTICE: S3 Object Lock retention is explicitly bypassed for"
          echo "non-production profile '${SPINE_DEPLOYMENT_PROFILE}' via SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1."
          echo "OPERATIONAL RISK: Remote archives are not immutable and can be overwritten or deleted."
          echo "================================================================================"
        else
          echo "NOTICE: S3 Object Lock retention is not configured (SPINE_BACKUP_OBJECT_LOCK_MODE='${SPINE_BACKUP_OBJECT_LOCK_MODE}')."
        fi
        ;;
      *)
        echo "================================================================================" >&2
        echo "FATAL: Unsupported SPINE_BACKUP_OBJECT_LOCK_MODE: '${SPINE_BACKUP_OBJECT_LOCK_MODE}'" >&2
        echo "Supported S3 Object Lock modes are 'COMPLIANCE' (mandatory for production) or" >&2
        echo "'GOVERNANCE' (non-production only). Disabling Object Lock is permitted only in" >&2
        echo "non-production profiles with SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE=1." >&2
        echo "================================================================================" >&2
        exit 1
        ;;
    esac
  fi
}

RELEASED=0
STAGING_DIR=""
PLAINTEXT_TAR=""

cleanup_backup() {
  local exit_code=$?
  if [ "${RELEASED}" -eq 0 ] && [ -n "${SPINE_ADMIN_TOKEN:-}" ]; then
    echo "NOTICE: Cleaning up — ensuring cluster maintenance barrier is released..." >&2
    curl -s -X POST "${SPINE_API_URL}/api/v1/admin/maintenance/release" \
      -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" > /dev/null 2>&1 || true
  fi

  local cleanup_failed=0
  if [ -n "${PLAINTEXT_TAR:-}" ] && [ -e "${PLAINTEXT_TAR}" ]; then
    echo "Purging unencrypted plaintext bundle from disk: ${PLAINTEXT_TAR}..." >&2
    if ! rm -rf "${PLAINTEXT_TAR}"; then
      echo "ERROR: Failed to remove plaintext bundle: ${PLAINTEXT_TAR}" >&2
      cleanup_failed=1
    fi
  fi

  if [ -n "${STAGING_DIR:-}" ] && [ -e "${STAGING_DIR}" ]; then
    echo "Purging unencrypted staging directory from disk: ${STAGING_DIR}..." >&2
    if ! rm -rf "${STAGING_DIR}"; then
      echo "ERROR: Failed to remove staging directory: ${STAGING_DIR}" >&2
      cleanup_failed=1
    fi
  fi

  if [ "${CLEANUP_TEMP_PASSPHRASE}" -eq 1 ] && [ -n "${RESOLVED_PASSPHRASE_FILE:-}" ] && [ -f "${RESOLVED_PASSPHRASE_FILE}" ]; then
    dd if=/dev/urandom of="${RESOLVED_PASSPHRASE_FILE}" bs=64 count=1 conv=notrunc 2>/dev/null || true
    rm -f "${RESOLVED_PASSPHRASE_FILE}"
  fi

  if [ "${cleanup_failed}" -ne 0 ]; then
    echo "WARNING: Cleanup of unencrypted backup artifacts encountered errors!" >&2
  fi
}
trap cleanup_backup EXIT ERR INT TERM

to_host_mount_path() {
  if command -v cygpath >/dev/null 2>&1; then
    cygpath -w "$1"
  else
    printf "%s" "$1"
  fi
}

# Resolve passphrase, verify supported storage topology, and enforce off-site policy before acquiring barrier
resolve_passphrase_file
verify_supported_storage_topology
verify_off_site_replication_policy

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
PLAINTEXT_TAR="${BACKUP_DIR}/.plaintext_bundle.tar.gz"
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
  spine-api backup encrypt \
    --passphrase-file "${RESOLVED_PASSPHRASE_FILE}" \
    --input "${PLAINTEXT_TAR}" \
    --output "${ENCRYPTED_ARCHIVE}"
else
  # Encrypt through the spine-api container via mounted read-only secret file.
  # Credentials are never passed in process arguments or container environment variables.
  mount_pass_file=$(to_host_mount_path "${RESOLVED_PASSPHRASE_FILE}")
  docker compose -f "${COMPOSE_FILE}" run --rm -i --no-deps \
    -v "${mount_pass_file}:/run/secrets/backup_passphrase:ro" \
    spine-api \
    backup encrypt --passphrase-file /run/secrets/backup_passphrase --input - --output - \
    < "${PLAINTEXT_TAR}" > "${ENCRYPTED_ARCHIVE}"
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

# Phase 2c: Off-Host Immutable Replication (S3 / Object Lock)
echo "=== Phase 2c: Off-Host Immutable Replication & Cryptographic Verification ==="
if [ -n "${SPINE_BACKUP_REMOTE_S3_URI}" ]; then
  REMOTE_BASE="${SPINE_BACKUP_REMOTE_S3_URI%/}"
  REMOTE_TARGET="${REMOTE_BASE}/$(basename "${BACKUP_DIR}")"
  echo "Replicating encrypted backup to remote object store: ${REMOTE_TARGET}..."

  AWS_EXTRA_ARGS=()
  if [ -n "${SPINE_S3_ENDPOINT_URL}" ]; then
    AWS_EXTRA_ARGS+=(--endpoint-url "${SPINE_S3_ENDPOINT_URL}")
  fi

  if ! command -v aws >/dev/null 2>&1; then
    echo "FATAL: SPINE_BACKUP_REMOTE_S3_URI specified, but 'aws' CLI is not available on host. Cannot complete off-host replication." >&2
    exit 1
  fi

  UPLOAD_ARGS=("${AWS_EXTRA_ARGS[@]}")
  APPLY_OBJECT_LOCK=0
  if [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "COMPLIANCE" ] || [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "GOVERNANCE" ]; then
    local profile
    profile=$(normalize_deployment_profile)
    if [ "${profile}" = "core-forge-v1" ] && [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "GOVERNANCE" ]; then
      echo "FATAL: S3 Object Lock mode 'GOVERNANCE' is rejected under production profile 'core-forge-v1'. Must be 'COMPLIANCE'." >&2
      exit 1
    fi
    RETAIN_UNTIL=$(compute_retain_until_date "${SPINE_BACKUP_RETENTION_DAYS}")
    if [ -z "${RETAIN_UNTIL}" ]; then
      echo "FATAL: S3 Object Lock retention is required (mode=${SPINE_BACKUP_OBJECT_LOCK_MODE}), but failed to compute retain-until date (+${SPINE_BACKUP_RETENTION_DAYS} days)." >&2
      exit 1
    fi
    echo "Applying S3 Object Lock retention: mode=${SPINE_BACKUP_OBJECT_LOCK_MODE}, retain_until=${RETAIN_UNTIL} (${SPINE_BACKUP_RETENTION_DAYS} days)"
    UPLOAD_ARGS+=(--object-lock-mode "${SPINE_BACKUP_OBJECT_LOCK_MODE}" --object-lock-retain-until-date "${RETAIN_UNTIL}")
    APPLY_OBJECT_LOCK=1
  elif [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "" ] || [ "${SPINE_BACKUP_OBJECT_LOCK_MODE}" = "NONE" ]; then
    local profile
    profile=$(normalize_deployment_profile)
    if [ "${profile}" = "core-forge-v1" ]; then
      echo "FATAL: S3 Object Lock retention cannot be omitted under production profile 'core-forge-v1'." >&2
      exit 1
    fi
    if [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "1" ] || [ "${SPINE_BACKUP_REQUIRE_REMOTE}" = "true" ]; then
      if [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" != "1" ] && [ "${SPINE_BACKUP_ALLOW_UNLOCKED_REMOTE}" != "true" ]; then
        echo "FATAL: S3 Object Lock retention is mandatory for production remote backups." >&2
        exit 1
      fi
    fi
  else
    echo "FATAL: Unsupported SPINE_BACKUP_OBJECT_LOCK_MODE: '${SPINE_BACKUP_OBJECT_LOCK_MODE}'. Must be 'COMPLIANCE' or 'GOVERNANCE'." >&2
    exit 1
  fi

  echo "Uploading backup.spnebak to ${REMOTE_TARGET}/backup.spnebak..."
  if ! aws s3 cp "${ENCRYPTED_ARCHIVE}" "${REMOTE_TARGET}/backup.spnebak" "${UPLOAD_ARGS[@]}"; then
    echo "FATAL: Failed to replicate backup archive to remote object store ${REMOTE_TARGET}/backup.spnebak" >&2
    exit 1
  fi

  echo "Uploading SHA256SUMS to ${REMOTE_TARGET}/SHA256SUMS..."
  if ! aws s3 cp "${BACKUP_DIR}/SHA256SUMS" "${REMOTE_TARGET}/SHA256SUMS" "${AWS_EXTRA_ARGS[@]}"; then
    echo "FATAL: Failed to replicate SHA256SUMS to remote object store ${REMOTE_TARGET}/SHA256SUMS" >&2
    exit 1
  fi

  echo "Verifying remote object size and cryptographic byte integrity..."
  LOCAL_SIZE=$(stat -c%s "${ENCRYPTED_ARCHIVE}" 2>/dev/null || stat -f%z "${ENCRYPTED_ARCHIVE}" 2>/dev/null || wc -c < "${ENCRYPTED_ARCHIVE}" | tr -d '[:space:]')
  REMOTE_SIZE=$(aws s3 ls "${REMOTE_TARGET}/backup.spnebak" "${AWS_EXTRA_ARGS[@]}" | awk '{print $3}' | tr -d '[:space:]' || echo "0")
  if [ -z "${REMOTE_SIZE}" ] || [ "${LOCAL_SIZE}" != "${REMOTE_SIZE}" ]; then
    echo "FATAL: Remote size mismatch for ${REMOTE_TARGET}/backup.spnebak: local=${LOCAL_SIZE}, remote=${REMOTE_SIZE}" >&2
    exit 1
  fi

  LOCAL_SHA256=$(sha256sum "${ENCRYPTED_ARCHIVE}" | awk '{print $1}')
  echo "Streaming remote object from ${REMOTE_TARGET}/backup.spnebak to verify SHA-256 digest..."
  REMOTE_SHA256=$(aws s3 cp "${REMOTE_TARGET}/backup.spnebak" - "${AWS_EXTRA_ARGS[@]}" | sha256sum | awk '{print $1}')
  if [ -z "${REMOTE_SHA256}" ] || [ "${LOCAL_SHA256}" != "${REMOTE_SHA256}" ]; then
    echo "FATAL: Cryptographic verification failed! Remote SHA-256 does not match local archive." >&2
    echo "  Local SHA-256:  ${LOCAL_SHA256}" >&2
    echo "  Remote SHA-256: ${REMOTE_SHA256}" >&2
    exit 1
  fi

  # Parse S3 bucket and object key for retention metadata verification
  NO_SCHEME="${REMOTE_TARGET#s3://}"
  S3_BUCKET="${NO_SCHEME%%/*}"
  if [ "${NO_SCHEME}" != "${S3_BUCKET}" ]; then
    S3_PREFIX="${NO_SCHEME#*/}"
    S3_KEY="${S3_PREFIX}/backup.spnebak"
  else
    S3_KEY="backup.spnebak"
  fi

  if [ "${APPLY_OBJECT_LOCK}" -eq 1 ]; then
    echo "Verifying S3 Object Lock retention metadata on remote object (${S3_BUCKET}/${S3_KEY})..."
    RETENTION_OUTPUT=$(aws s3api get-object-retention --bucket "${S3_BUCKET}" --key "${S3_KEY}" "${AWS_EXTRA_ARGS[@]}" 2>&1 || true)
    REMOTE_LOCK_MODE=$(echo "${RETENTION_OUTPUT}" | grep -o '"Mode"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"Mode"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || echo "")
    REMOTE_RETAIN_UNTIL=$(echo "${RETENTION_OUTPUT}" | grep -o '"RetainUntilDate"[[:space:]]*:[[:space:]]*"[^"]*"' | sed -E 's/.*"RetainUntilDate"[[:space:]]*:[[:space:]]*"([^"]*)".*/\1/' || echo "")

    if [ -z "${REMOTE_LOCK_MODE}" ] || [ "${REMOTE_LOCK_MODE}" != "${SPINE_BACKUP_OBJECT_LOCK_MODE}" ]; then
      echo "================================================================================" >&2
      echo "FATAL: S3 Object Lock retention verification failed for ${REMOTE_TARGET}/backup.spnebak!" >&2
      echo "  Expected lock mode:   ${SPINE_BACKUP_OBJECT_LOCK_MODE}" >&2
      echo "  Observed lock mode:   ${REMOTE_LOCK_MODE:-<absent>}" >&2
      echo "  Observed retain date: ${REMOTE_RETAIN_UNTIL:-<absent>}" >&2
      echo "  s3api raw response:   ${RETENTION_OUTPUT}" >&2
      echo "Object Lock must be confirmed by remote metadata before declaring backup success." >&2
      echo "================================================================================" >&2
      exit 1
    fi

    if [ -z "${REMOTE_RETAIN_UNTIL}" ]; then
      echo "================================================================================" >&2
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate is missing from remote object metadata." >&2
      echo "  s3api raw response:   ${RETENTION_OUTPUT}" >&2
      echo "================================================================================" >&2
      exit 1
    fi

    REMOTE_EPOCH=$(iso_to_epoch "${REMOTE_RETAIN_UNTIL}")
    if [ -z "${REMOTE_EPOCH}" ]; then
      echo "================================================================================" >&2
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' is malformed or unparseable." >&2
      echo "================================================================================" >&2
      exit 1
    fi

    NOW_EPOCH=$(date +%s)
    if [ "${REMOTE_EPOCH}" -le "${NOW_EPOCH}" ]; then
      echo "================================================================================" >&2
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' (epoch: ${REMOTE_EPOCH}) is expired or in the past (current epoch: ${NOW_EPOCH})." >&2
      echo "================================================================================" >&2
      exit 1
    fi

    # Require retention date to meet the requested retention window (allowing 300s clock skew buffer)
    MIN_REQUIRED_EPOCH=$(( NOW_EPOCH + (SPINE_BACKUP_RETENTION_DAYS * 86400) - 300 ))
    if [ "${REMOTE_EPOCH}" -lt "${MIN_REQUIRED_EPOCH}" ]; then
      echo "================================================================================" >&2
      echo "FATAL: S3 Object Lock retention verification failed: RetainUntilDate '${REMOTE_RETAIN_UNTIL}' is earlier than the configured retention period of ${SPINE_BACKUP_RETENTION_DAYS} days." >&2
      echo "  Configured retention days: ${SPINE_BACKUP_RETENTION_DAYS}" >&2
      echo "  Minimum required epoch:    ${MIN_REQUIRED_EPOCH}" >&2
      echo "  Observed retention epoch:  ${REMOTE_EPOCH}" >&2
      echo "================================================================================" >&2
      exit 1
    fi

    echo "Remote Object Lock metadata verified: mode=${REMOTE_LOCK_MODE}, retain_until=${REMOTE_RETAIN_UNTIL} (epoch: ${REMOTE_EPOCH} >= min: ${MIN_REQUIRED_EPOCH})."
    echo "Remote replication cryptographically verified [OK] (${REMOTE_SIZE} bytes, SHA-256: ${REMOTE_SHA256}, immutable retention active: ${REMOTE_LOCK_MODE} until ${REMOTE_RETAIN_UNTIL})."
  else
    echo "Remote replication cryptographically verified [OK] (${REMOTE_SIZE} bytes, SHA-256: ${REMOTE_SHA256}, standard replication without Object Lock)."
  fi
else
  case "${SPINE_BACKUP_REQUIRE_REMOTE}" in
    1|true)
      echo "FATAL: Off-site replication policy is ENFORCED (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE}), but upload was not configured." >&2
      exit 1
      ;;
    0|false)
      echo "================================================================================"
      echo "NOTICE: Off-host backup replication (SPINE_BACKUP_REMOTE_S3_URI) is not configured."
      echo "Operating in INTENTIONALLY LOCAL-ONLY mode (SPINE_BACKUP_REQUIRE_REMOTE=${SPINE_BACKUP_REQUIRE_REMOTE})."
      echo "Backup is stored solely on local host storage (${BACKUP_DIR})."
      echo "OPERATIONAL LIMITATION: In the event of catastrophic host destruction, storage controller"
      echo "failure, or host compromise, local-only backups will be unrecoverable."
      echo "================================================================================"
      ;;
    *)
      echo "FATAL: Invalid SPINE_BACKUP_REQUIRE_REMOTE value: '${SPINE_BACKUP_REQUIRE_REMOTE}'" >&2
      exit 1
      ;;
  esac
fi

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
