#!/usr/bin/env python3
"""
Unit Tests for Deployed Operations Verifier (scripts/verify_deployed_operations.py).
"""

import os
import sys
import tempfile
import time
import unittest
from typing import List, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import verify_deployed_operations
from verify_deployed_operations import OperationalCheckFailure


class TestDeployedOperationsVerifier(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()

    def tearDown(self):
        self.temp_dir.cleanup()

    # --- 1. Scheduler Tests ---
    def test_scheduler_systemd_active(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            if "systemctl" in cmd and "is-active" in cmd:
                return 0, "active\n", ""
            return 1, "", "not found"

        info = verify_deployed_operations.check_scheduler_status(mode="auto", runner=mock_runner)
        self.assertEqual(info["scheduler"], "systemd")
        self.assertTrue(info["active"])

    def test_scheduler_cron_active(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            return 1, "inactive\n", ""

        cron_path = os.path.join(self.temp_dir.name, "spine-backup.cron")
        with open(cron_path, "w", encoding="utf-8") as f:
            f.write("# cron test\n")

        info = verify_deployed_operations.check_scheduler_status(
            mode="auto", cron_file=cron_path, runner=mock_runner
        )
        self.assertEqual(info["scheduler"], "cron")
        self.assertTrue(info["active"])

    def test_scheduler_none_active_fails(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            return 1, "inactive\n", ""

        missing_cron = os.path.join(self.temp_dir.name, "nonexistent.cron")
        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_scheduler_status(
                mode="auto", cron_file=missing_cron, runner=mock_runner
            )
        self.assertIn("No active scheduler found", str(cm.exception))

    # --- 2. Freshness Tests ---
    def test_backup_freshness_missing_dir(self):
        missing_dir = os.path.join(self.temp_dir.name, "missing")
        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_backup_freshness(backup_root=missing_dir)
        self.assertIn("does not exist", str(cm.exception))

    def test_backup_freshness_zero_archives(self):
        empty_dir = os.path.join(self.temp_dir.name, "empty_backups")
        os.makedirs(empty_dir)
        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_backup_freshness(backup_root=empty_dir)
        self.assertIn("Zero backup archives", str(cm.exception))

    def test_backup_freshness_zero_byte_archive(self):
        backup_dir = os.path.join(self.temp_dir.name, "backups")
        os.makedirs(backup_dir)
        archive = os.path.join(backup_dir, "backup.spnebak")
        with open(archive, "wb") as f:
            pass  # 0 bytes

        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_backup_freshness(backup_root=backup_dir)
        self.assertIn("zero bytes", str(cm.exception))

    def test_backup_freshness_stale_archive(self):
        backup_dir = os.path.join(self.temp_dir.name, "backups")
        os.makedirs(backup_dir)
        archive = os.path.join(backup_dir, "backup.spnebak")
        with open(archive, "wb") as f:
            f.write(b"SAMPLE_BACKUP_DATA")

        # Fake current time 120 minutes later
        now = os.path.getmtime(archive) + (120 * 60)
        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_backup_freshness(
                backup_root=backup_dir,
                max_age_minutes=60,
                current_time=now,
            )
        self.assertIn("RPO SLA breached", str(cm.exception))

    def test_backup_freshness_success(self):
        backup_dir = os.path.join(self.temp_dir.name, "backups")
        os.makedirs(backup_dir)
        archive = os.path.join(backup_dir, "backup.spnebak")
        with open(archive, "wb") as f:
            f.write(b"SAMPLE_BACKUP_DATA_OK")

        now = os.path.getmtime(archive) + (15 * 60)  # 15 minutes old
        res = verify_deployed_operations.check_backup_freshness(
            backup_root=backup_dir,
            max_age_minutes=60,
            current_time=now,
        )
        self.assertEqual(res["latest_archive"], archive)
        self.assertEqual(res["size_bytes"], len(b"SAMPLE_BACKUP_DATA_OK"))
        self.assertAlmostEqual(res["age_minutes"], 15.0, delta=0.5)

    # --- 3. Alert Sink Tests ---
    def test_alert_sink_unconfigured(self):
        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_alert_sink(
                webhook_url=None, monitor_env_path="/nonexistent"
            )
        self.assertIn("ALERT_WEBHOOK_URL is unconfigured", str(cm.exception))

    def test_alert_sink_placeholder_rejected(self):
        placeholders = [
            "https://hooks.slack.com/services/XXX/YYY/ZZZ",
            "http://localhost:9000/webhook",
            "https://example.com/alerts",
            "<YOUR_WEBHOOK_URL>",
        ]
        for ph in placeholders:
            with self.subTest(placeholder=ph):
                with self.assertRaises(OperationalCheckFailure) as cm:
                    verify_deployed_operations.check_alert_sink(webhook_url=ph)
                self.assertIn("is a placeholder", str(cm.exception))

    def test_alert_sink_valid(self):
        res = verify_deployed_operations.check_alert_sink(
            webhook_url="https://hooks.slack.com/services/T0123/B0456/abcdef123456"
        )
        self.assertTrue(res["valid"])

    # --- 4. Journal and Logs Tests ---
    def test_journal_clean(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            return 0, "Nov 22 22:00:00 host spine-backup: backup completed in 42s\n", ""

        res = verify_deployed_operations.check_journal_and_logs(
            log_dir=self.temp_dir.name, runner=mock_runner
        )
        self.assertTrue(res["clean"])

    def test_journal_errors_flagged(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            return 0, "Nov 22 22:00:00 host spine-backup: FATAL: failed to freeze cluster\n", ""

        with self.assertRaises(OperationalCheckFailure) as cm:
            verify_deployed_operations.check_journal_and_logs(
                log_dir=self.temp_dir.name, runner=mock_runner
            )
        self.assertIn("error(s) in recent operational logs", str(cm.exception))

    # --- 5. End-to-End Test ---
    def test_verify_deployed_operations_end_to_end_success(self):
        def mock_runner(cmd: List[str]) -> Tuple[int, str, str]:
            if "is-active" in cmd:
                return 0, "active\n", ""
            if "journalctl" in cmd:
                return 0, "All services healthy\n", ""
            return 0, "", ""

        backup_dir = os.path.join(self.temp_dir.name, "backups")
        os.makedirs(backup_dir)
        archive = os.path.join(backup_dir, "backup.spnebak")
        with open(archive, "wb") as f:
            f.write(b"VALID_BACKUP_PAYLOAD")

        now = os.path.getmtime(archive) + (10 * 60)  # 10m old

        res = verify_deployed_operations.verify_deployed_operations(
            mode="systemd",
            backup_root=backup_dir,
            max_age_minutes=60,
            webhook_url="https://events.pagerduty.com/v2/enqueue",
            log_dir=self.temp_dir.name,
            runner=mock_runner,
            current_time=now,
        )
        self.assertEqual(res["status"], "DEPLOYED_OPERATIONAL_VERIFIED")


if __name__ == "__main__":
    unittest.main()
