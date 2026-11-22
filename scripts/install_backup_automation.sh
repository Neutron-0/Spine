#!/usr/bin/env bash
# ==============================================================================
# Spine Forge - Idempotent Backup & Freshness Automation Installer
#
# Configures automated hourly backups and 15-minute RPO SLA freshness
# monitoring on target Linux hosts via systemd timers or cron.
#
# Usage:
#   sudo ./scripts/install_backup_automation.sh [--systemd|--cron] [--dry-run]
# ==============================================================================
set -euo pipefail

SPINE_DIR="${SPINE_DIR:-/opt/spine}"
BACKUP_DIR="${BACKUP_DIR:-/mnt/backups/spine}"
LOG_DIR="${LOG_DIR:-/var/log/spine}"
CONF_DIR="${CONF_DIR:-/etc/spine}"
BACKUP_ENV="${CONF_DIR}/backup.env"
MONITOR_ENV="${CONF_DIR}/backup-monitor.env"

MODE="systemd"
DRY_RUN=0
INSTALLED_FILES=()

cleanup_on_error() {
  local exit_code=$?
  if [ "${DRY_RUN}" -eq 1 ]; then
    return
  fi
  echo "FATAL: Installation encountered an error (exit ${exit_code}). Rolling back partially installed components..." >&2
  for file in "${INSTALLED_FILES[@]}"; do
    if [ -f "${file}" ]; then
      echo "Rolling back: removing ${file}" >&2
      rm -f "${file}" || true
    fi
  done
  if [ "${MODE}" = "systemd" ] && command -v systemctl >/dev/null 2>&1; then
    systemctl daemon-reload 2>/dev/null || true
  fi
  echo "Rollback completed." >&2
  exit "${exit_code}"
}
trap cleanup_on_error ERR

while [ $# -gt 0 ]; do
  case "$1" in
    --systemd)
      MODE="systemd"
      shift
      ;;
    --cron)
      MODE="cron"
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --spine-dir)
      SPINE_DIR="$2"
      shift 2
      ;;
    --backup-dir)
      BACKUP_DIR="$2"
      shift 2
      ;;
    --log-dir)
      LOG_DIR="$2"
      shift 2
      ;;
    --conf-dir)
      CONF_DIR="$2"
      BACKUP_ENV="${CONF_DIR}/backup.env"
      MONITOR_ENV="${CONF_DIR}/backup-monitor.env"
      shift 2
      ;;
    -h|--help)
      echo "Usage: $0 [--systemd|--cron] [--dry-run] [--spine-dir DIR] [--backup-dir DIR] [--log-dir DIR] [--conf-dir DIR]"
      exit 0
      ;;
    *)
      echo "Unknown argument: $1" >&2
      exit 1
      ;;
  esac
done

execute() {
  if [ "${DRY_RUN}" -eq 1 ]; then
    echo "[DRY-RUN] $*"
  else
    "$@"
  fi
}

echo "=== Spine Forge Backup Automation Installer ==="
echo "Mode:              ${MODE}"
echo "Installation Dir:  ${SPINE_DIR}"
echo "Backup Target Dir: ${BACKUP_DIR}"
echo "Log Directory:     ${LOG_DIR}"
echo "Config Directory:  ${CONF_DIR}"
echo "Dry Run:           $([ ${DRY_RUN} -eq 1 ] && echo 'YES' || echo 'NO')"
echo "------------------------------------------------"

# 1. Root or sudo validation (unless dry-run)
if [ "${DRY_RUN}" -eq 0 ] && [ "$(id -u)" -ne 0 ]; then
  echo "FATAL: This installer must be executed as root (or via sudo)." >&2
  exit 1
fi

# 2. Preflight: Verify target backup and freshness scripts exist
SCRIPT_BACKUP="${SPINE_DIR}/scripts/backup_oob.sh"
SCRIPT_FRESHNESS="${SPINE_DIR}/scripts/check_backup_freshness.sh"

if [ ! -f "${SCRIPT_BACKUP}" ] || [ ! -f "${SCRIPT_FRESHNESS}" ]; then
  echo "WARNING: Target scripts not found at ${SPINE_DIR}/scripts."
  echo "Attempting to locate in current working directory..."
  CURRENT_DIR="$(pwd)"
  if [ -f "${CURRENT_DIR}/scripts/backup_oob.sh" ] && [ -f "${CURRENT_DIR}/scripts/check_backup_freshness.sh" ]; then
    echo "Found scripts in ${CURRENT_DIR}/scripts. Staging into ${SPINE_DIR}/scripts..."
    execute mkdir -p "${SPINE_DIR}/scripts"
    execute cp "${CURRENT_DIR}/scripts/backup_oob.sh" "${SPINE_DIR}/scripts/"
    execute cp "${CURRENT_DIR}/scripts/check_backup_freshness.sh" "${SPINE_DIR}/scripts/"
  else
    echo "FATAL: Unable to find backup_oob.sh and check_backup_freshness.sh." >&2
    exit 1
  fi
fi

execute chmod +x "${SPINE_DIR}/scripts/backup_oob.sh"
execute chmod +x "${SPINE_DIR}/scripts/check_backup_freshness.sh"

# 3. Create required runtime directories with restrictive permissions
execute mkdir -p "${BACKUP_DIR}"
execute chmod 0700 "${BACKUP_DIR}"
execute mkdir -p "${LOG_DIR}"
execute chmod 0750 "${LOG_DIR}"
execute mkdir -p "${CONF_DIR}"
execute chmod 0700 "${CONF_DIR}"

# 4. Validate or create environment configuration files
if [ ! -f "${BACKUP_ENV}" ]; then
  echo "Creating template ${BACKUP_ENV}..."
  if [ "${DRY_RUN}" -eq 1 ]; then
    echo "[DRY-RUN] touch ${BACKUP_ENV}"
  else
    cat << 'EOF' > "${BACKUP_ENV}"
# Spine Forge Backup Configuration
COMPOSE_FILE=/opt/spine/docker-compose.prod.yml
BACKUP_ROOT=/mnt/backups/spine
SPINE_ADMIN_TOKEN_FILE=/etc/spine/admin_token.secret
SPINE_BACKUP_PASSPHRASE_FILE=/etc/spine/backup_passphrase.secret
# Optional remote S3 offsite replication
# SPINE_BACKUP_REMOTE_S3_URI=s3://my-bucket/backups
# SPINE_BACKUP_OBJECT_LOCK_MODE=COMPLIANCE
EOF
    chmod 0600 "${BACKUP_ENV}"
  fi
else
  # Verify permission is 0600 or 0400
  ENV_PERM=$(stat -c "%a" "${BACKUP_ENV}" 2>/dev/null || stat -f "%OLp" "${BACKUP_ENV}" 2>/dev/null || echo "0600")
  if [ "${ENV_PERM}" != "600" ] && [ "${ENV_PERM}" != "400" ]; then
    echo "Hardening permissions on ${BACKUP_ENV} (current: ${ENV_PERM} -> target: 0600)..."
    execute chmod 0600 "${BACKUP_ENV}"
  fi
fi

if [ ! -f "${MONITOR_ENV}" ]; then
  echo "Creating template ${MONITOR_ENV}..."
  if [ "${DRY_RUN}" -eq 1 ]; then
    echo "[DRY-RUN] touch ${MONITOR_ENV}"
  else
    cat << 'EOF' > "${MONITOR_ENV}"
# Spine Forge Backup Freshness Monitor Configuration
BACKUP_ROOT=/mnt/backups/spine
MAX_AGE_MINUTES=60
# Set real operational webhook destination (Slack, PagerDuty, Discord, or Opsgenie)
ALERT_WEBHOOK_URL=
EOF
    chmod 0600 "${MONITOR_ENV}"
  fi
else
  ENV_PERM=$(stat -c "%a" "${MONITOR_ENV}" 2>/dev/null || stat -f "%OLp" "${MONITOR_ENV}" 2>/dev/null || echo "0600")
  if [ "${ENV_PERM}" != "600" ] && [ "${ENV_PERM}" != "400" ]; then
    echo "Hardening permissions on ${MONITOR_ENV} (current: ${ENV_PERM} -> target: 0600)..."
    execute chmod 0600 "${MONITOR_ENV}"
  fi
fi

# 5. Mode-Specific Installation
if [ "${MODE}" = "systemd" ]; then
  echo "--- Installing systemd timers and service units ---"
  SRC_DIR="deploy/systemd"
  if [ ! -d "${SRC_DIR}" ]; then
    SRC_DIR="${SPINE_DIR}/deploy/systemd"
  fi

  for unit in spine-backup.service spine-backup.timer spine-backup-freshness.service spine-backup-freshness.timer; do
    if [ -f "${SRC_DIR}/${unit}" ]; then
      execute cp "${SRC_DIR}/${unit}" "/etc/systemd/system/${unit}"
      execute chmod 0644 "/etc/systemd/system/${unit}"
      INSTALLED_FILES+=("/etc/systemd/system/${unit}")
      echo "Installed /etc/systemd/system/${unit}"
    else
      echo "FATAL: Could not locate ${SRC_DIR}/${unit}" >&2
      exit 1
    fi
  done

  execute systemctl daemon-reload
  execute systemctl enable --now spine-backup.timer
  execute systemctl enable --now spine-backup-freshness.timer

  if [ "${DRY_RUN}" -eq 0 ]; then
    echo "Verifying active systemd timers..."
    systemctl is-active spine-backup.timer || { echo "FATAL: spine-backup.timer failed to activate!" >&2; exit 1; }
    systemctl is-active spine-backup-freshness.timer || { echo "FATAL: spine-backup-freshness.timer failed to activate!" >&2; exit 1; }
    echo "[SUCCESS] Both systemd timers are active and scheduled."
  fi

elif [ "${MODE}" = "cron" ]; then
  echo "--- Installing cron schedule ---"
  SRC_CRON="deploy/cron/spine-backup.cron"
  if [ ! -f "${SRC_CRON}" ]; then
    SRC_CRON="${SPINE_DIR}/deploy/cron/spine-backup.cron"
  fi

  if [ -f "${SRC_CRON}" ]; then
    execute cp "${SRC_CRON}" "/etc/cron.d/spine-backup"
    execute chmod 0644 "/etc/cron.d/spine-backup"
    execute chown root:root "/etc/cron.d/spine-backup"
    INSTALLED_FILES+=("/etc/cron.d/spine-backup")
    echo "Installed /etc/cron.d/spine-backup"
  else
    echo "FATAL: Could not locate ${SRC_CRON}" >&2
    exit 1
  fi
fi

echo "=========================================================="
echo "Backup automation installed successfully in ${MODE} mode."
echo "Run scripts/verify_deployed_operations.py to validate status."
echo "=========================================================="
