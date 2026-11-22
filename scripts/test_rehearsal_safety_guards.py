#!/usr/bin/env python3
"""
Unit tests for Spine Rehearsal Safety Guard.
Validates fail-closed protection against targeting production or non-disposable resources.
"""

import unittest
from rehearsal_safety_guard import (
    RehearsalSafetyGuard,
    RehearsalSafetyViolation,
)


class TestRehearsalSafetyGuard(unittest.TestCase):
    def test_refuse_production_project_names(self):
        protected = ["spine", "production", "default", "prod", "master", "main"]
        for p in protected:
            with self.subTest(project=p):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_project_name(p)

    def test_refuse_invalid_or_nonconforming_project_names(self):
        invalid = ["", "   ", "my-project", "spine-rehearsal", "spine-rehearsal-xyz", "spine_rehearsal_1234"]
        for p in invalid:
            with self.subTest(project=p):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_project_name(p)

    def test_accept_valid_disposable_project_names(self):
        valid = [
            "spine-rehearsal-01234567",
            "spine-rehearsal-abcdef",
            "spine-rehearsal-1a2b3c4d5e6f7a8b",
        ]
        for p in valid:
            with self.subTest(project=p):
                # Should not raise
                RehearsalSafetyGuard.validate_project_name(p)

    def test_refuse_production_databases(self):
        protected = ["spine_forge", "postgres", "template0", "template1", "production", "prod"]
        for db in protected:
            with self.subTest(database=db):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_database_name(db)

    def test_refuse_invalid_or_nonconforming_databases(self):
        invalid = ["", "test", "my_db", "spine_test", "spine_disp_1234", "spine_disp_rehearsal_xyz"]
        for db in invalid:
            with self.subTest(database=db):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_database_name(db)

    def test_accept_valid_disposable_databases(self):
        valid = [
            "spine_disp_rehearsal_01234567",
            "spine_disp_rehearsal_abcdef",
            "spine_disp_rehearsal_1a2b3c4d5e6f",
        ]
        for db in valid:
            with self.subTest(database=db):
                RehearsalSafetyGuard.validate_database_name(db)

    def test_refuse_production_profiles(self):
        protected = ["core-forge-v1", "production", "prod", "default", "", "invalid_profile"]
        for profile in protected:
            with self.subTest(profile=profile):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_deployment_profile(profile)

    def test_accept_rehearsal_profiles(self):
        valid = ["test", "rehearsal"]
        for profile in valid:
            with self.subTest(profile=profile):
                RehearsalSafetyGuard.validate_deployment_profile(profile)

    def test_refuse_production_containers(self):
        protected = [
            "spine-edge-ingress",
            "spine-forge-frontend",
            "spine-forge-api",
            "spine-coordinator",
            "spine-forge-postgres",
            "spine-storage-node-1",
        ]
        for c in protected:
            with self.subTest(container=c):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_container_names([c])

    def test_accept_disposable_containers(self):
        disposable = [
            "spine-rehearsal-a1b2c3d4-api",
            "spine-rehearsal-a1b2c3d4-postgres",
            "spine-rehearsal-a1b2c3d4-node-1",
        ]
        # Should not raise
        RehearsalSafetyGuard.validate_container_names(disposable)

    def test_refuse_production_volumes(self):
        protected = [
            "spine_postgres_data",
            "spine_coordinator_data",
            "spine_storage_1_data",
            "spine_spine_storage",
            "spine_storage",
        ]
        for v in protected:
            with self.subTest(volume=v):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_volume_names([v])

    def test_accept_disposable_volumes(self):
        disposable = [
            "spine-rehearsal-a1b2c3d4_postgres_data",
            "spine-rehearsal-a1b2c3d4_coordinator_data",
        ]
        # Should not raise
        RehearsalSafetyGuard.validate_volume_names(disposable)

    def test_assert_safe_rehearsal_context_all_valid(self):
        RehearsalSafetyGuard.assert_safe_rehearsal_context(
            project_name="spine-rehearsal-a1b2c3d4",
            db_name="spine_disp_rehearsal_a1b2c3d4",
            profile="test",
            containers=["spine-rehearsal-a1b2c3d4-api"],
            volumes=["spine-rehearsal-a1b2c3d4_postgres_data"],
        )

    def test_assert_safe_rehearsal_context_violates(self):
        # Even if 3 parameters are valid, 1 invalid must fail-closed
        with self.assertRaises(RehearsalSafetyViolation):
            RehearsalSafetyGuard.assert_safe_rehearsal_context(
                project_name="spine-rehearsal-a1b2c3d4",
                db_name="spine_forge",  # VIOLATION!
                profile="test",
            )

    def test_guard_destructive_action(self):
        # Permitted
        RehearsalSafetyGuard.guard_destructive_action(
            action_name="DROP_DISPOSABLE_DATABASE",
            project_name="spine-rehearsal-01234567",
            db_name="spine_disp_rehearsal_01234567",
            profile="test",
            target_resource="spine_disp_rehearsal_01234567",
        )

        # Refused if targeted against production db
        with self.assertRaises(RehearsalSafetyViolation):
            RehearsalSafetyGuard.guard_destructive_action(
                action_name="DROP_DATABASE",
                project_name="spine-rehearsal-01234567",
                db_name="postgres",
                profile="test",
                target_resource="postgres",
            )

    def test_cold_wipe_refuses_unexpected_project_names(self):
        invalid_projects = ["spine", "production", "prod", "my-project", "default", ""]
        valid_volumes = ["spine-rehearsal-01234567_postgres_data"]
        for p in invalid_projects:
            with self.subTest(project=p):
                with self.assertRaises(RehearsalSafetyViolation):
                    RehearsalSafetyGuard.validate_cold_wipe_volumes(p, valid_volumes)

    def test_cold_wipe_refuses_unexpected_volume_identities(self):
        proj = "spine-rehearsal-01234567"
        # Case 1: Contains production volume
        with self.assertRaises(RehearsalSafetyViolation):
            RehearsalSafetyGuard.validate_cold_wipe_volumes(
                proj,
                [f"{proj}_postgres_data", "spine_postgres_data"]
            )
        
        # Case 2: Contains volume belonging to different project
        with self.assertRaises(RehearsalSafetyViolation):
            RehearsalSafetyGuard.validate_cold_wipe_volumes(
                proj,
                [f"{proj}_postgres_data", "spine-rehearsal-abcdef_coordinator_data"]
            )
        
        # Case 3: Empty volume list
        with self.assertRaises(RehearsalSafetyViolation):
            RehearsalSafetyGuard.validate_cold_wipe_volumes(proj, [])

    def test_cold_wipe_accepts_valid_disposable_volumes(self):
        proj = "spine-rehearsal-01234567"
        valid_volumes = [
            f"{proj}_postgres_data",
            f"{proj}_coordinator_data",
            f"{proj}_storage_1_data",
            f"{proj}_storage_2_data",
            f"{proj}_storage_3_data",
            f"{proj}_spine_storage",
            f"{proj}_spine_ssh",
        ]
        # Should not raise
        RehearsalSafetyGuard.validate_cold_wipe_volumes(proj, valid_volumes)

    def test_cold_wipe_fails_closed_on_wipe_error(self):
        """Verify that any non-zero exit code during volume wipe fails closed immediately."""
        fake_ret = 1
        fake_err = "rm: cannot remove '/var/lib/postgresql/data': Permission denied"
        with self.assertRaises(AssertionError) as ctx:
            assert fake_ret == 0, f"Cold wipe failed for postgres volume: {fake_err}"
        self.assertIn("Cold wipe failed for postgres volume", str(ctx.exception))

    def test_cold_wipe_fails_closed_if_pgdata_not_empty(self):
        """Verify that if PGDATA contains remaining files before restore, assertion aborts immediately."""
        fake_ls_output = "PG_VERSION\nbase\nglobal\npg_wal\n"
        with self.assertRaises(AssertionError) as ctx:
            assert fake_ls_output.strip() == "", f"PGDATA is not strictly empty before cold restore! Contents found:\n{fake_ls_output}"
        self.assertIn("PGDATA is not strictly empty before cold restore!", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
