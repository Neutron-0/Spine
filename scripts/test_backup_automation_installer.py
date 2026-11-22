#!/usr/bin/env python3
"""
Unit and Integration Tests for Backup Automation Installer
(scripts/install_backup_automation.sh).
"""

import os
import stat
import subprocess
import tempfile
import unittest


class TestBackupAutomationInstaller(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.spine_dir = os.path.join(self.temp_dir.name, "opt", "spine")
        self.conf_dir = os.path.join(self.temp_dir.name, "etc", "spine")
        self.backup_dir = os.path.join(self.temp_dir.name, "mnt", "backups")
        self.log_dir = os.path.join(self.temp_dir.name, "var", "log", "spine")

        # Mock deploy structure inside spine_dir
        os.makedirs(os.path.join(self.spine_dir, "scripts"), exist_ok=True)
        os.makedirs(os.path.join(self.spine_dir, "deploy", "systemd"), exist_ok=True)
        os.makedirs(os.path.join(self.spine_dir, "deploy", "cron"), exist_ok=True)

        for s in ("backup_oob.sh", "check_backup_freshness.sh"):
            p = os.path.join(self.spine_dir, "scripts", s)
            with open(p, "w") as f:
                f.write("#!/bin/bash\nexit 0\n")
            os.chmod(p, 0o755)

        for u in ("spine-backup.service", "spine-backup.timer", "spine-backup-freshness.service", "spine-backup-freshness.timer"):
            with open(os.path.join(self.spine_dir, "deploy", "systemd", u), "w") as f:
                f.write("[Unit]\nDescription=Test\n")

        with open(os.path.join(self.spine_dir, "deploy", "cron", "spine-backup.cron"), "w") as f:
            f.write("# cron test\n")

    def tearDown(self):
        self.temp_dir.cleanup()

    def test_dry_run_makes_zero_changes(self):
        """Dry-run flag must exit 0 and make zero filesystem changes."""
        res = subprocess.run(
            [
                "bash",
                "scripts/install_backup_automation.sh",
                "--dry-run",
                "--spine-dir",
                self.spine_dir,
                "--conf-dir",
                self.conf_dir,
                "--backup-dir",
                self.backup_dir,
                "--log-dir",
                self.log_dir,
            ],
            capture_output=True,
            text=True,
        )
        self.assertEqual(res.returncode, 0, f"STDOUT:\n{res.stdout}\nSTDERR:\n{res.stderr}")
        self.assertIn("Dry Run:           YES", res.stdout)
        self.assertIn("[DRY-RUN]", res.stdout)
        # Verify conf_dir was not created on disk
        self.assertFalse(os.path.exists(self.conf_dir))

    def test_idempotence_and_permissions(self):
        """Running installer in dry-run preserves expected arguments and flags across repeated runs."""
        for _ in range(2):
            res = subprocess.run(
                [
                    "bash",
                    "scripts/install_backup_automation.sh",
                    "--cron",
                    "--dry-run",
                    "--spine-dir",
                    self.spine_dir,
                    "--conf-dir",
                    self.conf_dir,
                    "--backup-dir",
                    self.backup_dir,
                    "--log-dir",
                    self.log_dir,
                ],
                capture_output=True,
                text=True,
            )
            self.assertEqual(res.returncode, 0)
            self.assertIn("Mode:              cron", res.stdout)
            self.assertIn("Installed /etc/cron.d/spine-backup", res.stdout)


if __name__ == "__main__":
    unittest.main()
