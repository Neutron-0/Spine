#!/usr/bin/env python3
"""
Unit Tests for S3 Compliance WORM Rehearsal Tool (scripts/rehearse_s3_compliance.py).
"""

import datetime
import json
import os
import sys
import unittest
from typing import Dict, List, Tuple

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import rehearse_s3_compliance
from rehearse_s3_compliance import (
    PositiveControlFailure,
    RehearsalError,
    WORMEnforcementBreach,
)


class MockAWSCommandRunner:
    """Mock runner that intercepts AWS CLI command invocations."""

    def __init__(self):
        self.commands_issued: List[List[str]] = []
        self.control_object_uploaded = False
        self.control_object_deleted = False
        self.fail_identity = False
        self.fail_iam_sim = False
        self.fail_control_delete = False
        self.locked_delete_succeeds = False  # Critical failure simulation
        self.locked_delete_unrelated_error = False
        self.wrong_retention_mode = False
        self.past_retention_date = False

    def __call__(self, cmd: List[str]) -> Tuple[int, str, str]:
        self.commands_issued.append(cmd)
        cmd_str = " ".join(cmd)

        # sts get-caller-identity
        if "sts get-caller-identity" in cmd_str:
            if self.fail_identity:
                return 0, json.dumps({"Arn": "arn:aws:iam::123456789012:role/UntrustedRole"}), ""
            return 0, json.dumps({"Arn": "arn:aws:sts::123456789012:assumed-role/SpineForge-AuditRole/session"}), ""

        # iam simulate-principal-policy
        if "iam simulate-principal-policy" in cmd_str:
            decision = "implicitDeny" if self.fail_iam_sim else "allowed"
            return 0, json.dumps({
                "EvaluationResults": [
                    {"EvalActionName": "s3:DeleteObject", "EvalDecision": decision},
                    {"EvalActionName": "s3:DeleteObjectVersion", "EvalDecision": decision},
                ]
            }), ""

        # s3api get-bucket-policy
        if "s3api get-bucket-policy" in cmd_str:
            return 0, json.dumps({"Policy": json.dumps({"Statement": []})}, default=str), ""

        # s3api put-object (control object)
        if "s3api put-object" in cmd_str:
            self.control_object_uploaded = True
            return 0, json.dumps({"VersionId": "ctrl-v1"}), ""

        # s3api head-object
        if "s3api head-object" in cmd_str:
            if "control-delete-test.txt" in cmd_str:
                return 0, json.dumps({"VersionId": "ctrl-v1", "ContentLength": 100}), ""
            # Target object
            return 0, json.dumps({"VersionId": "tgt-v1", "ContentLength": 1024000}), ""

        # s3api delete-object
        if "s3api delete-object" in cmd_str:
            if "control-delete-test.txt" in cmd_str:
                if self.fail_control_delete:
                    return 1, "", "An error occurred (AccessDenied) when calling the DeleteObject operation: Access Denied"
                self.control_object_deleted = True
                return 0, json.dumps({"VersionId": "ctrl-v1"}), ""
            # Target object delete attempt
            if self.locked_delete_succeeds:
                return 0, json.dumps({"VersionId": "tgt-v1"}), ""
            if self.locked_delete_unrelated_error:
                return 1, "", "An error occurred (NoSuchKey) when calling the DeleteObject operation: The specified key does not exist."
            # Normal expected WORM refusal:
            return 1, "", "An error occurred (AccessDenied) when calling the DeleteObject operation: Access Denied (Object Lock COMPLIANCE mode retention active)"

        # s3api get-object-retention
        if "s3api get-object-retention" in cmd_str:
            mode = "GOVERNANCE" if self.wrong_retention_mode else "COMPLIANCE"
            now = datetime.datetime.now(datetime.timezone.utc)
            if self.past_retention_date:
                retain_date = (now - datetime.timedelta(days=1)).isoformat()
            else:
                retain_date = (now + datetime.timedelta(days=365)).isoformat()
            return 0, json.dumps({
                "Retention": {
                    "Mode": mode,
                    "RetainUntilDate": retain_date,
                }
            }), ""

        return 0, "{}", ""


class TestS3ComplianceRehearsal(unittest.TestCase):
    def setUp(self):
        self.runner = MockAWSCommandRunner()

    def test_rehearsal_happy_path(self):
        """Complete 5-phase rehearsal succeeds with valid AWS responses."""
        res = rehearse_s3_compliance.rehearse_s3_compliance(
            bucket="spineforge-backups-production",
            target_key="backups/postgres-20261122.tar.gz",
            target_version_id="tgt-v1",
            expected_role="SpineForge-AuditRole",
            runner=self.runner,
        )
        self.assertTrue(res)
        self.assertTrue(self.runner.control_object_uploaded)
        self.assertTrue(self.runner.control_object_deleted)

    def test_phase1_identity_mismatch_fails(self):
        """Fails when assumed role does not match expected role substring."""
        self.runner.fail_identity = True
        with self.assertRaises(RehearsalError) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("does not match expected role", str(cm.exception))

    def test_phase2_iam_permission_denied_fails(self):
        """Fails when principal policy simulation shows DeleteObject is not allowed."""
        self.runner.fail_iam_sim = True
        with self.assertRaises(RehearsalError) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("must possess valid deletion permissions", str(cm.exception))

    def test_phase3_control_object_deletion_fails(self):
        """Fails when unlocked control object cannot be deleted."""
        self.runner.fail_control_delete = True
        with self.assertRaises(PositiveControlFailure) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("Control object deletion failed", str(cm.exception))

    def test_phase4_locked_object_deleted_critical_breach(self):
        """CRITICAL: Fails immediately if S3 permits deletion of the locked object."""
        self.runner.locked_delete_succeeds = True
        with self.assertRaises(WORMEnforcementBreach) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("CRITICAL IMMUTABILITY BREACH", str(cm.exception))

    def test_phase4_locked_object_unrelated_error_fails_closed(self):
        """Fails closed if deletion error is not AccessDenied (e.g. NoSuchKey)."""
        self.runner.locked_delete_unrelated_error = True
        with self.assertRaises(RehearsalError) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("Expected AccessDenied", str(cm.exception))

    def test_phase5_retention_mode_mismatch_fails(self):
        """Fails if retention mode is GOVERNANCE instead of COMPLIANCE."""
        self.runner.wrong_retention_mode = True
        with self.assertRaises(RehearsalError) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("Expected Object Lock Mode 'COMPLIANCE'", str(cm.exception))

    def test_phase5_retention_date_expired_fails(self):
        """Fails if RetainUntilDate is in the past."""
        self.runner.past_retention_date = True
        with self.assertRaises(RehearsalError) as cm:
            rehearse_s3_compliance.rehearse_s3_compliance(
                bucket="spineforge-backups-production",
                target_key="backups/postgres-20261122.tar.gz",
                target_version_id="tgt-v1",
                expected_role="SpineForge-AuditRole",
                runner=self.runner,
            )
        self.assertIn("is in the past", str(cm.exception))


if __name__ == "__main__":
    unittest.main()
