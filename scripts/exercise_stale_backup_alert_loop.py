#!/usr/bin/env python3
"""
Exercise Stale Backup Alerting Loop.
Validates the end-to-end RPO monitoring loop by:
1. Spinning up a local ephemeral HTTP webhook server.
2. Inducing an RPO breach with an aged (>60m) backup archive.
3. Running scripts/check_backup_freshness.sh against it.
4. Verifying check_backup_freshness.sh exits with failure (1).
5. Capturing and asserting the exact alert webhook JSON payload.
6. Verifying that a fresh backup passes cleanly without firing alerts.
"""

import http.server
import json
import os
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def find_bash():
    git_bash = r"C:\Program Files\Git\bin\bash.exe"
    if os.name == "nt" and os.path.exists(git_bash):
        return git_bash
    which_bash = shutil.which("bash")
    if which_bash:
        return which_bash
    return "bash"


class WebhookHandler(http.server.BaseHTTPRequestHandler):
    received_payloads = []

    def do_POST(self):
        content_length = int(self.headers.get("Content-Length", 0))
        post_data = self.rfile.read(content_length)
        try:
            parsed = json.loads(post_data.decode("utf-8"))
        except Exception:
            parsed = {"raw": post_data.decode("utf-8", errors="replace")}
        WebhookHandler.received_payloads.append(parsed)
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.end_headers()
        self.wfile.write(b'{"status":"received"}')

    def log_message(self, format, *args):
        # Suppress noisy HTTP request logging
        pass


def run_freshness_check(backup_root: str, max_age_minutes: int, webhook_url: str):
    bash_bin = find_bash()
    script_path = (REPO_ROOT / "scripts" / "check_backup_freshness.sh").as_posix()
    env = os.environ.copy()
    env["BACKUP_ROOT"] = backup_root
    env["MAX_AGE_MINUTES"] = str(max_age_minutes)
    env["ALERT_WEBHOOK_URL"] = webhook_url

    res = subprocess.run(
        [bash_bin, script_path],
        capture_output=True,
        text=True,
        env=env,
    )
    return res


def main():
    print("=== Exercise Stale Backup Alerting Loop ===")
    server = http.server.HTTPServer(("127.0.0.1", 0), WebhookHandler)
    port = server.server_port
    server_thread = threading.Thread(target=server.serve_forever, daemon=True)
    server_thread.start()
    webhook_url = f"http://127.0.0.1:{port}/alert"
    print(f"[*] Ephemeral webhook listener running at: {webhook_url}")

    with tempfile.TemporaryDirectory(prefix="spine-alert-rehearsal-") as tmpdir:
        backup_file = Path(tmpdir) / "backup.spnebak"
        backup_file.write_bytes(b"MOCK_SPINE_BACKUP_PAYLOAD_FOR_ALERT_TEST")

        # ----------------------------------------------------------------------
        # Test 1: Fresh backup (1 minute old) -> Should PASS, zero alerts
        # ----------------------------------------------------------------------
        print("\n--- Test 1: Fresh backup (<5m old) ---")
        now = time.time()
        os.utime(backup_file, (now - 60, now - 60))
        res = run_freshness_check(tmpdir, 60, webhook_url)
        print(f"Return code: {res.returncode}")
        print(f"Stdout:\n{res.stdout.strip()}")
        assert res.returncode == 0, f"Expected 0 for fresh backup, got {res.returncode}"
        assert len(WebhookHandler.received_payloads) == 0, f"Expected 0 alerts for fresh backup, got {len(WebhookHandler.received_payloads)}"
        print("[PASS] Fresh backup passed with 0 alerts fired.")

        # ----------------------------------------------------------------------
        # Test 2: Induced Stale Backup (120 minutes old) -> Should FAIL, fire webhook
        # ----------------------------------------------------------------------
        print("\n--- Test 2: Induced Stale Backup (120m old, RPO threshold=60m) ---")
        stale_time = now - (120 * 60)
        os.utime(backup_file, (stale_time, stale_time))
        res = run_freshness_check(tmpdir, 60, webhook_url)
        print(f"Return code: {res.returncode}")
        print(f"Stderr:\n{res.stderr.strip()}")
        assert res.returncode == 1, f"Expected 1 for stale backup, got {res.returncode}"
        assert len(WebhookHandler.received_payloads) == 1, f"Expected 1 webhook payload, got {len(WebhookHandler.received_payloads)}"
        alert = WebhookHandler.received_payloads[0]
        print(f"[+] Received Alert Webhook Payload:\n{json.dumps(alert, indent=2)}")
        assert "RPO SLA Breached" in alert.get("text", ""), f"Alert payload missing RPO SLA breach message: {alert}"
        assert "120m old" in alert.get("text", "") or "119m old" in alert.get("text", "") or "121m old" in alert.get("text", "")
        print("[PASS] Stale backup induced SLA alert captured and verified.")

        # ----------------------------------------------------------------------
        # Test 3: Zero-Byte Backup Archive -> Should FAIL, fire webhook
        # ----------------------------------------------------------------------
        print("\n--- Test 3: Induced Corrupt Zero-Byte Archive ---")
        backup_file.write_bytes(b"")
        res = run_freshness_check(tmpdir, 60, webhook_url)
        print(f"Return code: {res.returncode}")
        print(f"Stderr:\n{res.stderr.strip()}")
        assert res.returncode == 1, f"Expected 1 for zero-byte backup, got {res.returncode}"
        assert len(WebhookHandler.received_payloads) == 2, f"Expected 2 total webhook payloads, got {len(WebhookHandler.received_payloads)}"
        zero_alert = WebhookHandler.received_payloads[1]
        print(f"[+] Received Zero-Byte Alert Payload:\n{json.dumps(zero_alert, indent=2)}")
        assert "zero-byte empty" in zero_alert.get("text", "")
        print("[PASS] Corrupt backup induced SLA alert captured and verified.")

    server.shutdown()
    print("\n[SUCCESS] End-to-end RPO monitoring and alert loop verified.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
