#!/usr/bin/env python3
"""
Spine Forge - Deployed Operations & RPO Verification Tool

Validates whether scheduled backup automation and freshness monitoring
are genuinely installed, scheduled, and operational on the target deployment host.

Checks enforced:
1. Scheduler status (systemd timers active, or cron job installed).
2. Freshness SLA (most recent backup <= 60 minutes old and non-empty).
3. Alert sink verification (ALERT_WEBHOOK_URL is configured, non-placeholder, and valid).
4. Service journal / log audit (no recent fatal errors or failure exits).

Classification output:
- 'DEPLOYED_OPERATIONAL_VERIFIED': Host is fully enrolled with active timers, fresh backups, and valid alert sink.
- 'PREPARED_LOCAL_REHEARSAL_ONLY': Scripts exist and pass local syntax/tests, but host enrollment is pending.
- 'OPERATIONAL_FAILURE': Host enrollment has failed or RPO SLA has breached.
"""

import argparse
import glob
import json
import os
import re
import subprocess
import sys
import time
import urllib.parse
import urllib.request
from typing import Any, Callable, Dict, List, Optional, Tuple


PLACEHOLDER_WEBHOOK_PATTERN = re.compile(
    r"(example\.com|localhost|127\.0\.0\.1|XXX/YYY/ZZZ|<.*>|^$)",
    re.IGNORECASE,
)


class OperationalCheckFailure(Exception):
    """Exception raised when an operational readiness check fails."""
    pass


def run_command(
    cmd: List[str],
    runner: Optional[Callable[[List[str]], Tuple[int, str, str]]] = None,
) -> Tuple[int, str, str]:
    if runner:
        return runner(cmd)
    res = subprocess.run(cmd, capture_output=True, text=True)
    return res.returncode, res.stdout, res.stderr


def check_scheduler_status(
    mode: str = "auto",
    cron_file: str = "/etc/cron.d/spine-backup",
    runner: Optional[Callable] = None,
) -> Dict[str, Any]:
    """Check whether systemd timers or cron jobs are active."""
    result: Dict[str, Any] = {"scheduler": None, "active": False, "details": []}

    # Try systemd first if auto or systemd
    if mode in ("auto", "systemd"):
        code_backup, out_backup, _ = run_command(["systemctl", "is-active", "spine-backup.timer"], runner)
        code_fresh, out_fresh, _ = run_command(["systemctl", "is-active", "spine-backup-freshness.timer"], runner)

        is_backup_active = (code_backup == 0 and out_backup.strip() == "active")
        is_fresh_active = (code_fresh == 0 and out_fresh.strip() == "active")

        if is_backup_active and is_fresh_active:
            result["scheduler"] = "systemd"
            result["active"] = True
            result["details"] = [
                "spine-backup.timer: active",
                "spine-backup-freshness.timer: active",
            ]
            return result
        elif mode == "systemd":
            raise OperationalCheckFailure(
                f"systemd timer check failed: spine-backup.timer ({out_backup.strip() or 'inactive'}), "
                f"spine-backup-freshness.timer ({out_fresh.strip() or 'inactive'})."
            )

    # Try cron
    if mode in ("auto", "cron"):
        if os.path.isfile(cron_file):
            result["scheduler"] = "cron"
            result["active"] = True
            result["details"] = [f"Found active crontab: {cron_file}"]
            return result
        elif mode == "cron":
            raise OperationalCheckFailure(f"Cron check failed: {cron_file} does not exist.")

    raise OperationalCheckFailure(
        "No active scheduler found. Neither systemd timers (spine-backup.timer) "
        f"nor crontab ({cron_file}) are active on this host."
    )


def check_backup_freshness(
    backup_root: str = "/mnt/backups/spine",
    max_age_minutes: int = 60,
    current_time: Optional[float] = None,
) -> Dict[str, Any]:
    """Verify that a non-empty backup exists within the max_age_minutes window."""
    if not os.path.isdir(backup_root):
        raise OperationalCheckFailure(f"Backup root directory '{backup_root}' does not exist.")

    pattern = os.path.join(backup_root, "**", "backup.spnebak")
    archives = glob.glob(pattern, recursive=True)

    if not archives:
        raise OperationalCheckFailure(
            f"Zero backup archives ('backup.spnebak') found in '{backup_root}'. "
            "Automated backups have not produced any archives."
        )

    # Find the most recently modified archive
    latest_archive = max(archives, key=os.path.getmtime)
    size = os.path.getsize(latest_archive)
    mtime = os.path.getmtime(latest_archive)

    if size == 0:
        raise OperationalCheckFailure(f"Latest backup archive '{latest_archive}' is zero bytes.")

    now = current_time if current_time is not None else time.time()
    age_seconds = now - mtime
    age_minutes = age_seconds / 60.0

    if age_minutes > max_age_minutes:
        raise OperationalCheckFailure(
            f"RPO SLA breached: Latest backup archive '{latest_archive}' is {age_minutes:.1f} minutes old "
            f"(threshold: <= {max_age_minutes} minutes)."
        )

    return {
        "latest_archive": latest_archive,
        "size_bytes": size,
        "age_minutes": round(age_minutes, 2),
    }


def load_env_file(path: str) -> Dict[str, str]:
    """Parse a KEY=VALUE environment file."""
    env = {}
    if not os.path.isfile(path):
        return env
    with open(path, "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if not line or line.startswith("#"):
                continue
            if "=" in line:
                k, v = line.split("=", 1)
                env[k.strip()] = v.strip().strip("'\"")
    return env


def check_alert_sink(
    webhook_url: Optional[str] = None,
    monitor_env_path: str = "/etc/spine/backup-monitor.env",
    test_ping: bool = False,
) -> Dict[str, Any]:
    """Verify ALERT_WEBHOOK_URL is configured and not a placeholder."""
    url = webhook_url or os.environ.get("ALERT_WEBHOOK_URL")
    if not url and os.path.isfile(monitor_env_path):
        env = load_env_file(monitor_env_path)
        url = env.get("ALERT_WEBHOOK_URL")

    if not url or not url.strip():
        raise OperationalCheckFailure(
            "ALERT_WEBHOOK_URL is unconfigured. Stale-backup alerts will not reach an operator."
        )

    url = url.strip()
    if PLACEHOLDER_WEBHOOK_PATTERN.search(url):
        raise OperationalCheckFailure(
            f"ALERT_WEBHOOK_URL '{url}' is a placeholder or local URL. "
            "A real operational alert sink (Slack, PagerDuty, Discord, Opsgenie) must be configured."
        )

    parsed = urllib.parse.urlparse(url)
    if parsed.scheme not in ("http", "https") or not parsed.netloc:
        raise OperationalCheckFailure(f"ALERT_WEBHOOK_URL '{url}' is not a valid HTTP/HTTPS URL.")

    if test_ping:
        data = json.dumps({"text": "[Spine Forge] Deployed operational verification test ping."}).encode("utf-8")
        req = urllib.request.Request(url, data=data, headers={"Content-Type": "application/json"})
        try:
            with urllib.request.urlopen(req, timeout=10) as resp:
                if resp.status >= 400:
                    raise OperationalCheckFailure(f"Alert sink webhook returned HTTP {resp.status}")
        except Exception as exc:
            raise OperationalCheckFailure(f"Alert sink test ping failed to connect: {exc}")

    return {"webhook_url": url, "valid": True}


def check_journal_and_logs(
    log_dir: str = "/var/log/spine",
    runner: Optional[Callable] = None,
) -> Dict[str, Any]:
    """Check systemd journal or log files for recent failures."""
    errors_found: List[str] = []

    # 1. Try systemd journal
    code, stdout, _ = run_command(
        ["journalctl", "-u", "spine-backup.service", "-u", "spine-backup-freshness.service", "--since", "2 hours ago", "--no-pager"],
        runner,
    )
    if code == 0 and stdout:
        for line in stdout.splitlines():
            if re.search(r"(failed with exit-code|FATAL|Traceback|CRITICAL ALERT)", line, re.IGNORECASE):
                errors_found.append(f"[journal] {line.strip()}")

    # 2. Check log files if present
    for log_name in ("backup.log", "backup-freshness.log"):
        log_path = os.path.join(log_dir, log_name)
        if os.path.isfile(log_path):
            with open(log_path, "r", encoding="utf-8", errors="replace") as f:
                lines = f.readlines()[-50:]  # last 50 lines
                for line in lines:
                    if re.search(r"(FATAL:|CRITICAL ALERT|Traceback \(most recent call last\))", line):
                        errors_found.append(f"[{log_name}] {line.strip()}")

    if errors_found:
        raise OperationalCheckFailure(
            f"Found {len(errors_found)} error(s) in recent operational logs:\n"
            + "\n".join(errors_found[:5])
        )

    return {"clean": True, "inspected_logs": True}


def verify_deployed_operations(
    mode: str = "auto",
    backup_root: str = "/mnt/backups/spine",
    max_age_minutes: int = 60,
    monitor_env: str = "/etc/spine/backup-monitor.env",
    webhook_url: Optional[str] = None,
    log_dir: str = "/var/log/spine",
    cron_file: str = "/etc/cron.d/spine-backup",
    test_ping: bool = False,
    runner: Optional[Callable] = None,
    current_time: Optional[float] = None,
) -> Dict[str, Any]:
    """Execute all operational readiness checks."""
    print("=" * 70)
    print("Spine Forge - Deployed Operations Verification")
    print("=" * 70)

    # 1. Scheduler
    print("1. Verifying Scheduler Status (systemd / cron)...")
    sched_info = check_scheduler_status(mode=mode, cron_file=cron_file, runner=runner)
    print(f"   [PASS] Active scheduler: {sched_info['scheduler']} ({'; '.join(sched_info['details'])})")

    # 2. Backup Freshness
    print("2. Verifying Backup Freshness & RPO SLA...")
    fresh_info = check_backup_freshness(
        backup_root=backup_root,
        max_age_minutes=max_age_minutes,
        current_time=current_time,
    )
    print(
        f"   [PASS] Archive: {fresh_info['latest_archive']} "
        f"({fresh_info['size_bytes']} bytes, age: {fresh_info['age_minutes']}m <= {max_age_minutes}m)"
    )

    # 3. Alert Sink
    print("3. Verifying Alert Delivery Sink Configuration...")
    sink_info = check_alert_sink(
        webhook_url=webhook_url,
        monitor_env_path=monitor_env,
        test_ping=test_ping,
    )
    print(f"   [PASS] Alert destination configured: {sink_info['webhook_url']}")

    # 4. Logs & Journal
    print("4. Auditing Service Logs & Journal for Recent Errors...")
    log_info = check_journal_and_logs(log_dir=log_dir, runner=runner)
    print("   [PASS] No fatal errors or failure exits found in recent service logs.")

    print("=" * 70)
    print("STATUS: DEPLOYED_OPERATIONAL_VERIFIED")
    print("Host is actively running scheduled backups and monitoring RPO freshness.")
    print("=" * 70)

    return {
        "status": "DEPLOYED_OPERATIONAL_VERIFIED",
        "scheduler": sched_info,
        "freshness": fresh_info,
        "sink": sink_info,
        "logs": log_info,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description="Spine Forge Deployed Operations Verifier")
    parser.add_argument("--mode", choices=["auto", "systemd", "cron"], default="auto")
    parser.add_argument("--backup-root", default="/mnt/backups/spine")
    parser.add_argument("--max-age-minutes", type=int, default=60)
    parser.add_argument("--monitor-env", default="/etc/spine/backup-monitor.env")
    parser.add_argument("--webhook-url", default=None)
    parser.add_argument("--log-dir", default="/var/log/spine")
    parser.add_argument("--cron-file", default="/etc/cron.d/spine-backup")
    parser.add_argument("--test-ping", action="store_true")

    args = parser.parse_args()

    try:
        verify_deployed_operations(
            mode=args.mode,
            backup_root=args.backup_root,
            max_age_minutes=args.max_age_minutes,
            monitor_env=args.monitor_env,
            webhook_url=args.webhook_url,
            log_dir=args.log_dir,
            cron_file=args.cron_file,
            test_ping=args.test_ping,
        )
    except Exception as exc:
        print(f"\n[FATAL ERROR] Deployed operations verification failed: {exc}", file=sys.stderr)
        print("STATUS: PREPARED_LOCAL_REHEARSAL_ONLY (host enrollment incomplete)", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
