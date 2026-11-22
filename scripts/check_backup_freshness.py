#!/usr/bin/env python3
"""
Spine Forge - Production Backup Freshness & RPO SLA Monitor
Cross-platform monitor verifying that the most recent backup archive
satisfies the Recovery Point Objective (RPO) SLA of <= 60 minutes.
"""

import os
import sys
import time
from pathlib import Path

def check_freshness(backup_root: Path, max_age_minutes: int = 60) -> tuple[bool, str, int]:
    """
    Checks the latest backup in backup_root against max_age_minutes.
    Returns: (is_healthy, message, exit_code)
    """
    if not backup_root.exists() or not backup_root.is_dir():
        return False, f"[CRITICAL ALERT] RPO SLA Breached: Backup directory '{backup_root}' does not exist!", 2

    # Find backup.spnebak files
    candidates = list(backup_root.glob("**/backup.spnebak"))
    if not candidates:
        return False, f"[CRITICAL ALERT] RPO SLA Breached: Zero valid 'backup.spnebak' archives found in '{backup_root}'!", 2

    # Find latest by mtime
    candidates.sort(key=lambda p: p.stat().st_mtime, reverse=True)
    latest = candidates[0]
    st = latest.stat()

    if st.st_size == 0:
        return False, f"[CRITICAL ALERT] RPO SLA Breached: Latest archive '{latest}' is 0-byte empty!", 1

    current_time = time.time()
    age_seconds = current_time - st.st_mtime
    age_minutes = age_seconds / 60.0
    size_mb = st.st_size / (1024 * 1024)

    status_str = (
        f"=== Spine Backup Freshness Audit ===\n"
        f"  Backup Path:   {latest}\n"
        f"  Archive Size:  {size_mb:.2f} MB\n"
        f"  Backup Age:    {age_minutes:.1f} minutes ({age_seconds:.0f}s)\n"
        f"  RPO Threshold: {max_age_minutes} minutes"
    )

    if age_minutes > max_age_minutes:
        msg = (
            f"{status_str}\n"
            f"[CRITICAL ALERT] RPO SLA Breached! Latest backup is {age_minutes:.1f}m old "
            f"(exceeds {max_age_minutes}m threshold)."
        )
        return False, msg, 1

    msg = (
        f"{status_str}\n"
        f"[OK] RPO SLA Met: Latest backup is fresh ({age_minutes:.1f}m old <= {max_age_minutes}m threshold)."
    )
    return True, msg, 0


def main():
    backup_root_env = os.environ.get("BACKUP_ROOT", "/mnt/backups/spine")
    max_age_env = int(os.environ.get("MAX_AGE_MINUTES", "60"))
    backup_root = Path(backup_root_env)

    ok, message, code = check_freshness(backup_root, max_age_env)
    print(message)
    sys.exit(code)


if __name__ == "__main__":
    main()
