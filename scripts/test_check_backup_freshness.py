#!/usr/bin/env python3
"""
Unit tests for Spine Backup Freshness & RPO Monitor.
"""

import tempfile
import time
import unittest
from pathlib import Path
from check_backup_freshness import check_freshness


class TestCheckBackupFreshness(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory()
        self.root = Path(self.temp_dir.name)

    def tearDown(self):
        self.temp_dir.cleanup()

    def test_missing_directory(self):
        non_existent = self.root / "does_not_exist"
        ok, msg, code = check_freshness(non_existent, max_age_minutes=60)
        self.assertFalse(ok)
        self.assertEqual(code, 2)
        self.assertIn("does not exist", msg)

    def test_no_backups(self):
        ok, msg, code = check_freshness(self.root, max_age_minutes=60)
        self.assertFalse(ok)
        self.assertEqual(code, 2)
        self.assertIn("Zero valid 'backup.spnebak' archives found", msg)

    def test_empty_backup(self):
        gen_dir = self.root / "gen_1"
        gen_dir.mkdir()
        backup = gen_dir / "backup.spnebak"
        backup.write_bytes(b"")
        ok, msg, code = check_freshness(self.root, max_age_minutes=60)
        self.assertFalse(ok)
        self.assertEqual(code, 1)
        self.assertIn("0-byte empty", msg)

    def test_fresh_backup(self):
        gen_dir = self.root / "gen_1"
        gen_dir.mkdir()
        backup = gen_dir / "backup.spnebak"
        backup.write_bytes(b"SPNEBAK_SAMPLE_DATA")
        ok, msg, code = check_freshness(self.root, max_age_minutes=60)
        self.assertTrue(ok)
        self.assertEqual(code, 0)
        self.assertIn("[OK] RPO SLA Met", msg)

    def test_stale_backup(self):
        gen_dir = self.root / "gen_1"
        gen_dir.mkdir()
        backup = gen_dir / "backup.spnebak"
        backup.write_bytes(b"SPNEBAK_SAMPLE_DATA")
        # Set mtime to 90 minutes ago
        past_time = time.time() - (90 * 60)
        import os
        os.utime(backup, (past_time, past_time))

        ok, msg, code = check_freshness(self.root, max_age_minutes=60)
        self.assertFalse(ok)
        self.assertEqual(code, 1)
        self.assertIn("[CRITICAL ALERT] RPO SLA Breached", msg)


if __name__ == "__main__":
    unittest.main()
