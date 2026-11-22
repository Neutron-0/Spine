#!/usr/bin/env python3
"""
Spine Production Safety Guard for Destructive Rehearsals
Enforces fail-closed protection against accidental mutation, data wipe,
or disaster recovery execution on production or persistent resources.
"""

import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Iterable, Set

PRODUCTION_PROJECT_NAMES: Set[str] = {
    "spine",
    "production",
    "default",
    "prod",
    "master",
    "main",
}

PRODUCTION_DATABASES: Set[str] = {
    "spine_forge",
    "postgres",
    "template0",
    "template1",
    "production",
    "prod",
}

PRODUCTION_CONTAINERS: Set[str] = {
    "spine-edge-ingress",
    "spine-forge-frontend",
    "spine-forge-api",
    "spine-coordinator",
    "spine-forge-postgres",
    "spine-storage-node-1",
    "spine-storage-node-2",
    "spine-storage-node-3",
}

PRODUCTION_VOLUMES: Set[str] = {
    "spine_postgres_data",
    "spine_coordinator_data",
    "spine_storage_1_data",
    "spine_storage_2_data",
    "spine_storage_3_data",
    "spine_spine_storage",
    "spine_spine_ssh",
    "spine_storage",
}

DISPOSABLE_PROJECT_REGEX = re.compile(r"^spine-rehearsal-[0-9a-fA-F]{6,16}$")
DISPOSABLE_DB_REGEX = re.compile(r"^spine_disp_rehearsal_[0-9a-fA-F]{6,16}$")


class RehearsalSafetyViolation(Exception):
    """Raised when a destructive rehearsal safety check fails."""
    pass


class RehearsalSafetyGuard:
    """Validates that destructive operations operate strictly on disposable resources."""

    @staticmethod
    def validate_project_name(project_name: str) -> None:
        if not project_name or not isinstance(project_name, str):
            raise RehearsalSafetyViolation("Project name cannot be empty or non-string.")
        
        normalized = project_name.strip().lower()
        if normalized in PRODUCTION_PROJECT_NAMES:
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Refusing to target protected/production Compose project '{project_name}'."
            )
        
        if not DISPOSABLE_PROJECT_REGEX.match(normalized):
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Compose project '{project_name}' does not match disposable pattern 'spine-rehearsal-<hex>'."
            )

    @staticmethod
    def validate_database_name(db_name: str) -> None:
        if not db_name or not isinstance(db_name, str):
            raise RehearsalSafetyViolation("Database name cannot be empty or non-string.")
        
        normalized = db_name.strip().lower()
        if normalized in PRODUCTION_DATABASES:
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Refusing to target protected/production database '{db_name}'."
            )
        
        if not DISPOSABLE_DB_REGEX.match(normalized):
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Database '{db_name}' does not match disposable pattern 'spine_disp_rehearsal_<hex>'."
            )

    @staticmethod
    def validate_deployment_profile(profile: str) -> None:
        if not profile or not isinstance(profile, str):
            raise RehearsalSafetyViolation("Deployment profile cannot be empty.")
        
        normalized = profile.strip().lower()
        if normalized in ("core-forge-v1", "production", "prod"):
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Refusing to run destructive rehearsal under production profile '{profile}'."
            )
        
        if normalized not in ("test", "rehearsal", "development"):
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Unsupported rehearsal profile '{profile}'. Must be 'test' or 'rehearsal'."
            )

    @staticmethod
    def validate_container_names(containers: Iterable[str]) -> None:
        for c in containers:
            if not c or not isinstance(c, str):
                continue
            normalized = c.strip().lower()
            if normalized in PRODUCTION_CONTAINERS:
                raise RehearsalSafetyViolation(
                    f"SAFETY REFUSAL: Target container '{c}' is a protected production container."
                )

    @staticmethod
    def validate_volume_names(volumes: Iterable[str]) -> None:
        for v in volumes:
            if not v or not isinstance(v, str):
                continue
            normalized = v.strip().lower()
            if normalized in PRODUCTION_VOLUMES:
                raise RehearsalSafetyViolation(
                    f"SAFETY REFUSAL: Target volume '{v}' is a protected production volume."
                )

    @staticmethod
    def validate_docker_context() -> str:
        try:
            res = subprocess.run(
                ["docker", "context", "show"],
                capture_output=True,
                text=True,
                check=False
            )
            ctx = res.stdout.strip().lower() if res.returncode == 0 else "default"
        except Exception:
            ctx = "default"
        
        if any(prod_word in ctx for prod_word in ["prod", "production", "live", "k8s-prod"]):
            raise RehearsalSafetyViolation(
                f"SAFETY REFUSAL: Current Docker context '{ctx}' appears to be a production environment."
            )
        return ctx

    @classmethod
    def assert_safe_rehearsal_context(
        cls,
        project_name: str,
        db_name: str,
        profile: str,
        containers: Iterable[str] | None = None,
        volumes: Iterable[str] | None = None,
    ) -> None:
        """Atomic validation gate verifying all rehearsal parameters are disposable."""
        cls.validate_docker_context()
        cls.validate_project_name(project_name)
        cls.validate_database_name(db_name)
        cls.validate_deployment_profile(profile)
        if containers:
            cls.validate_container_names(containers)
        if volumes:
            cls.validate_volume_names(volumes)

    @classmethod
    def guard_destructive_action(
        cls,
        action_name: str,
        project_name: str,
        db_name: str,
        profile: str,
        target_resource: str,
    ) -> None:
        """Enforces validation immediately prior to executing any destructive action."""
        cls.assert_safe_rehearsal_context(project_name, db_name, profile)
        print(f"[SAFETY GUARD] Action '{action_name}' permitted on disposable resource: {target_resource}")
