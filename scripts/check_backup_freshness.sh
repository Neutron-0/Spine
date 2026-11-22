#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Production Backup Freshness & RPO SLA Monitor
# Enforces RPO SLA (threshold <= 60 minutes) by verifying recent backup archives.
# Designed for execution via systemd timer or cron monitoring alerts.
# ==============================================================================
set -euo pipefail

BACKUP_ROOT="${BACKUP_ROOT:-/mnt/backups/spine}"
MAX_AGE_MINUTES="${MAX_AGE_MINUTES:-60}"
ALERT_WEBHOOK_URL="${ALERT_WEBHOOK_URL:-}"

send_alert() {
  local msg="$1"
  echo "${msg}" >&2
  if [ -n "${ALERT_WEBHOOK_URL}" ]; then
    local payload
    payload=$(python3 -c "import json, sys; print(json.dumps({'text': sys.argv[1]}))" "${msg}" 2>/dev/null || printf '{"text": "%s"}' "${msg}")
    curl -s --connect-timeout 5 --max-time 10 -X POST -H "Content-Type: application/json" -d "${payload}" "${ALERT_WEBHOOK_URL}" || true
  fi
}

if [ ! -d "${BACKUP_ROOT}" ]; then
  send_alert "[CRITICAL ALERT] RPO SLA Breached: Backup directory '${BACKUP_ROOT}' does not exist!"
  exit 2
fi

# Locate the most recent backup archive
LATEST_BACKUP=$(python3 -c "
import os, glob, sys
backups = glob.glob(os.path.join(sys.argv[1], '**', 'backup.spnebak'), recursive=True)
if backups:
    latest = max(backups, key=os.path.getmtime)
    print(latest)
" "${BACKUP_ROOT}" 2>/dev/null || true)

if [ -z "${LATEST_BACKUP}" ]; then
  LATEST_BACKUP=$(find "${BACKUP_ROOT}" -type f -name "backup.spnebak" 2>/dev/null | head -n1 || true)
fi

if [ -z "${LATEST_BACKUP}" ] || [ ! -f "${LATEST_BACKUP}" ]; then
  send_alert "[CRITICAL ALERT] RPO SLA Breached: Zero valid 'backup.spnebak' archives found in '${BACKUP_ROOT}'!"
  exit 2
fi

# Check file size
FILE_SIZE=$(wc -c < "${LATEST_BACKUP}" | tr -d '[:space:]')
if [ "${FILE_SIZE}" -eq 0 ]; then
  send_alert "[CRITICAL ALERT] RPO SLA Breached: Latest backup archive '${LATEST_BACKUP}' is zero-byte empty!"
  exit 1
fi

# Compute age in seconds
CURRENT_TIME=$(date +%s)
FILE_MTIME=$(stat -c %Y "${LATEST_BACKUP}" 2>/dev/null || stat -f %m "${LATEST_BACKUP}" 2>/dev/null || echo "0")

if [ "${FILE_MTIME}" -eq 0 ]; then
  send_alert "[ERROR] Unable to determine modification timestamp for '${LATEST_BACKUP}'."
  exit 2
fi

AGE_SECONDS=$((CURRENT_TIME - FILE_MTIME))
AGE_MINUTES=$((AGE_SECONDS / 60))
MAX_AGE_SECONDS=$((MAX_AGE_MINUTES * 60))

echo "=== Spine Backup Freshness Audit ==="
echo "  Backup Path:     ${LATEST_BACKUP}"
echo "  Archive Size:    $((FILE_SIZE / 1024 / 1024)) MB"
echo "  Backup Age:      ${AGE_MINUTES} minutes (${AGE_SECONDS} seconds)"
echo "  RPO Threshold:   ${MAX_AGE_MINUTES} minutes"

if [ "${AGE_SECONDS}" -gt "${MAX_AGE_SECONDS}" ]; then
  MSG="[CRITICAL ALERT] RPO SLA Breached! Latest backup is ${AGE_MINUTES}m old (exceeds ${MAX_AGE_MINUTES}m threshold). Immediate operator intervention required."
  send_alert "${MSG}"
  exit 1
fi

echo "[OK] RPO SLA Met: Latest backup is fresh (${AGE_MINUTES}m old <= ${MAX_AGE_MINUTES}m threshold)."
exit 0
