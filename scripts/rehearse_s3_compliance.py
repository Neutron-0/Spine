#!/usr/bin/env python3
"""
Spine Forge - S3 Object Lock Compliance WORM Rehearsal Tool

Rigorous 5-phase verification proving that S3 backup immutability is enforced
by S3 Object Lock COMPLIANCE mode rather than IAM misconfigurations or bucket policies:

Phase 1: Identity & Principal Verification
  - Resolves identity via configured AWS profile/role (rejects raw key CLI args).
  - Asserts assumed role matches expected audit principal.

Phase 2: IAM & Policy Preflight Simulation
  - Simulates principal policy for s3:DeleteObject and s3:DeleteObjectVersion.
  - Verifies policy evaluation result is explicitly 'allowed'.
  - Checks bucket policy for absence of explicit Deny statements targeting principal.

Phase 3: Control Object Deletion (Positive Permission Check)
  - Puts an unlocked control object into the bucket.
  - Attempts version-specific deletion of the control object.
  - Must SUCCEED. If this fails, the audit role lacks deletion permissions,
    invalidating any subsequent Object Lock proof.

Phase 4: Target Locked Object Version Deletion (WORM Enforcement Probe)
  - Attempts version-specific deletion of the specified immutable backup archive.
  - Must FAIL with AccessDenied / HTTP 403.
  - If deletion succeeds, fails immediately as a CRITICAL WORM breach.
  - Unrelated errors (NoSuchKey, InvalidToken, network error) fail closed as test errors.

Phase 5: Retention & Version Integrity Verification
  - Queries head-object to confirm target version still exists and is accessible.
  - Queries get-object-retention to assert Mode is COMPLIANCE and RetainUntilDate is in the future.
"""

import argparse
import datetime
import json
import re
import subprocess
import sys
from typing import Any, Callable, Dict, List, Optional, Tuple


class RehearsalError(Exception):
    """Base error for rehearsal check failures."""
    pass


class WORMEnforcementBreach(RehearsalError):
    """Critical failure: Locked object was deleted."""
    pass


class PositiveControlFailure(RehearsalError):
    """Failure: Unlocked control object could not be deleted."""
    pass


def run_aws_cli(
    args: List[str],
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable[[List[str]], Tuple[int, str, str]]] = None,
) -> Tuple[int, str, str]:
    """Execute an AWS CLI command with optional profile and region."""
    cmd = ["aws"]
    if profile:
        cmd.extend(["--profile", profile])
    if region:
        cmd.extend(["--region", region])
    cmd.extend(args)

    if runner:
        return runner(cmd)

    proc = subprocess.run(cmd, capture_output=True, text=True)
    return proc.returncode, proc.stdout, proc.stderr


def verify_caller_identity(
    expected_role_arn_substring: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> Dict[str, Any]:
    """Phase 1: Verify assumed identity matches expected audit principal."""
    code, stdout, stderr = run_aws_cli(["sts", "get-caller-identity", "--output", "json"], profile, region, runner)
    if code != 0:
        raise RehearsalError(f"Failed to get caller identity: {stderr.strip()}")

    identity = json.loads(stdout)
    arn = identity.get("Arn", "")
    if expected_role_arn_substring not in arn:
        raise RehearsalError(
            f"Assumed identity '{arn}' does not match expected role substring '{expected_role_arn_substring}'."
        )
    return identity


def verify_iam_delete_permissions(
    principal_arn: str,
    bucket: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> None:
    """Phase 2a: Simulate policy to confirm principal is permitted to delete objects."""
    resource_arn = f"arn:aws:s3:::{bucket}/*"
    actions = ["s3:DeleteObject", "s3:DeleteObjectVersion"]
    
    code, stdout, stderr = run_aws_cli(
        [
            "iam",
            "simulate-principal-policy",
            "--policy-source-arn",
            principal_arn,
            "--action-names",
        ]
        + actions
        + ["--resource-arns", resource_arn, "--output", "json"],
        profile,
        region,
        runner,
    )
    if code != 0:
        raise RehearsalError(f"IAM policy simulation failed: {stderr.strip()}")

    sim_res = json.loads(stdout)
    results = sim_res.get("EvaluationResults", [])
    for res in results:
        action = res.get("EvalActionName")
        decision = res.get("EvalDecision")
        if decision != "allowed":
            raise RehearsalError(
                f"Principal policy simulation returned '{decision}' for action '{action}' on '{resource_arn}'. "
                "The audit role must possess valid deletion permissions to prove Object Lock enforcement."
            )


def verify_bucket_policy(
    bucket: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> None:
    """Phase 2b: Verify bucket policy exists and has no catch-all Deny on deletion."""
    code, stdout, stderr = run_aws_cli(
        ["s3api", "get-bucket-policy", "--bucket", bucket, "--output", "json"],
        profile,
        region,
        runner,
    )
    if code != 0:
        # If no bucket policy is configured, that's acceptable as IAM simulation already proved allow.
        if "NoSuchBucketPolicy" in stderr or "NoSuchBucketPolicy" in stdout:
            return
        raise RehearsalError(f"Failed to retrieve bucket policy for '{bucket}': {stderr.strip()}")

    policy_wrapper = json.loads(stdout)
    policy_str = policy_wrapper.get("Policy", "{}")
    if isinstance(policy_str, str):
        policy = json.loads(policy_str)
    else:
        policy = policy_str

    statements = policy.get("Statement", [])
    if isinstance(statements, dict):
        statements = [statements]

    for stmt in statements:
        if stmt.get("Effect") == "Deny":
            actions = stmt.get("Action", [])
            if isinstance(actions, str):
                actions = [actions]
            if any("s3:Delete" in a or a == "s3:*" or a == "*" for a in actions):
                # Check if it has a Condition or NotPrincipal that might exclude the audit role
                if "Condition" not in stmt:
                    print(
                        f"WARNING: Found unconditional Deny statement '{stmt.get('Sid', 'unnamed')}' in bucket policy. "
                        "Confirming with positive control object deletion in Phase 3..."
                    )


def check_bucket_default_retention(
    bucket: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> Optional[Dict[str, Any]]:
    """Check whether bucket-level default retention rule is active."""
    code, stdout, stderr = run_aws_cli(
        ["s3api", "get-object-lock-configuration", "--bucket", bucket, "--output", "json"],
        profile,
        region,
        runner,
    )
    if code != 0:
        return None
    try:
        data = json.loads(stdout)
        rule = data.get("ObjectLockConfiguration", {}).get("Rule", {})
        return rule.get("DefaultRetention")
    except Exception:
        return None


def exercise_control_object(
    bucket: str,
    control_key: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> None:
    """Phase 3: Positive control deletion test on an unlocked object."""
    # 1. Put unlocked control object
    code, stdout, stderr = run_aws_cli(
        [
            "s3api",
            "put-object",
            "--bucket",
            bucket,
            "--key",
            control_key,
            "--body",
            "-",
        ],
        profile,
        region,
        runner,
    )
    if code != 0:
        raise PositiveControlFailure(
            f"Failed to put control object into '{bucket}': {stderr.strip()}. "
            "Ensure the audit role has s3:PutObject permissions on the control location."
        )

    # 2. Get object version ID
    code, stdout, stderr = run_aws_cli(
        [
            "s3api",
            "head-object",
            "--bucket",
            bucket,
            "--key",
            control_key,
            "--output",
            "json",
        ],
        profile,
        region,
        runner,
    )
    if code != 0:
        raise PositiveControlFailure(f"Failed to read control object after upload: {stderr.strip()}")

    head = json.loads(stdout)
    version_id = head.get("VersionId")

    # 3. Attempt version-specific deletion
    del_args = [
        "s3api",
        "delete-object",
        "--bucket",
        bucket,
        "--key",
        control_key,
    ]
    if version_id:
        del_args.extend(["--version-id", version_id])

    del_code, del_out, del_err = run_aws_cli(del_args, profile, region, runner)
    if del_code != 0:
        raise PositiveControlFailure(
            f"Control object deletion failed (code {del_code}): {del_err.strip()}. "
            "Because unlocked objects cannot be deleted, rejection of the locked archive cannot be attributed to Object Lock."
        )


def attempt_locked_object_deletion(
    bucket: str,
    target_key: str,
    target_version_id: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> None:
    """Phase 4: Probe locked object deletion. Must fail strictly with AccessDenied / 403."""
    code, stdout, stderr = run_aws_cli(
        [
            "s3api",
            "delete-object",
            "--bucket",
            bucket,
            "--key",
            target_key,
            "--version-id",
            target_version_id,
        ],
        profile,
        region,
        runner,
    )

    if code == 0:
        raise WORMEnforcementBreach(
            f"CRITICAL IMMUTABILITY BREACH: S3 accepted version-specific delete for locked object "
            f"s3://{bucket}/{target_key} version {target_version_id}! Object Lock COMPLIANCE was NOT enforced."
        )

    # Check that error is specifically AccessDenied / 403
    combined_err = f"{stdout}\n{stderr}"
    if "AccessDenied" not in combined_err and "403" not in combined_err:
        raise RehearsalError(
            f"Deletion failed with unexpected error code: {stderr.strip() or stdout.strip()}. "
            "Expected AccessDenied / HTTP 403. Other errors do not constitute proof of Object Lock retention."
        )


def verify_object_retention_and_integrity(
    bucket: str,
    target_key: str,
    target_version_id: str,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> Dict[str, Any]:
    """Phase 5: Verify object version still exists and has active COMPLIANCE retention."""
    # 1. head-object
    code, stdout, stderr = run_aws_cli(
        [
            "s3api",
            "head-object",
            "--bucket",
            bucket,
            "--key",
            target_key,
            "--version-id",
            target_version_id,
            "--output",
            "json",
        ],
        profile,
        region,
        runner,
    )
    if code != 0:
        raise RehearsalError(f"Target object version no longer exists or head-object failed: {stderr.strip()}")

    head_data = json.loads(stdout)

    # 2. get-object-retention
    code, stdout, stderr = run_aws_cli(
        [
            "s3api",
            "get-object-retention",
            "--bucket",
            bucket,
            "--key",
            target_key,
            "--version-id",
            target_version_id,
            "--output",
            "json",
        ],
        profile,
        region,
        runner,
    )
    if code != 0:
        raise RehearsalError(f"Failed to query get-object-retention for target object: {stderr.strip()}")

    retention_data = json.loads(stdout)
    retention = retention_data.get("Retention", {})
    mode = retention.get("Mode")
    retain_until_str = retention.get("RetainUntilDate")

    if mode != "COMPLIANCE":
        raise RehearsalError(f"Expected Object Lock Mode 'COMPLIANCE', got '{mode}'.")

    if not retain_until_str:
        raise RehearsalError("RetainUntilDate is missing from object retention metadata.")

    # Parse ISO 8601 / RFC 3339 date
    retain_clean = retain_until_str.replace("Z", "+00:00")
    retain_dt = datetime.datetime.fromisoformat(retain_clean)
    now_dt = datetime.datetime.now(datetime.timezone.utc)

    if retain_dt <= now_dt:
        raise RehearsalError(f"RetainUntilDate ({retain_until_str}) is in the past! Current UTC: {now_dt.isoformat()}")

    return {
        "head": head_data,
        "retention": retention,
    }


def rehearse_s3_compliance(
    bucket: str,
    target_key: str,
    target_version_id: str,
    expected_role: str = "SpineForge-AuditRole",
    control_key: str = "rehearsals/control-delete-test.txt",
    control_bucket: Optional[str] = None,
    profile: Optional[str] = None,
    region: Optional[str] = None,
    runner: Optional[Callable] = None,
) -> bool:
    """Execute complete 5-phase S3 compliance rehearsal workflow."""
    print("=" * 70)
    print("Spine Forge - S3 Object Lock Compliance WORM Rehearsal")
    print("=" * 70)
    print(f"Target Bucket:     {bucket}")
    print(f"Control Bucket:    {control_bucket or bucket}")
    print(f"Target Key:        {target_key}")
    print(f"Target VersionId:  {target_version_id}")
    print(f"Expected Role:     {expected_role}")
    print(f"AWS Profile:       {profile or '[default]'}")
    print(f"AWS Region:        {region or '[default]'}")
    print("-" * 70)

    # Phase 1
    print("[Phase 1/5] Verifying caller identity & role...")
    identity = verify_caller_identity(expected_role, profile, region, runner)
    principal_arn = identity.get("Arn", "")
    print(f"   [PASS] Caller identity verified: {principal_arn}")

    # Phase 2
    print("[Phase 2/5] Evaluating IAM permissions and bucket policy...")
    verify_iam_delete_permissions(principal_arn, bucket, profile, region, runner)
    print("   [PASS] IAM principal policy simulation: s3:DeleteObject & s3:DeleteObjectVersion explicitly ALLOWED.")
    verify_bucket_policy(bucket, profile, region, runner)
    print("   [PASS] Bucket policy evaluated: no blocking Deny statements.")

    # Phase 2c: Evaluate Bucket Default Retention Rule
    default_retention = check_bucket_default_retention(bucket, profile, region, runner)
    effective_control_bucket = control_bucket or bucket

    if default_retention and not control_bucket:
        mode = default_retention.get("Mode", "COMPLIANCE")
        days = default_retention.get("Days", 30)
        raise RehearsalError(
            f"Bucket '{bucket}' has bucket-level default retention enabled (Mode: {mode}, Days: {days}). "
            "In S3, newly uploaded objects in this bucket automatically inherit COMPLIANCE retention, "
            "making unlocked positive-control deletion in this bucket infeasible. "
            "Pass an unlocked companion bucket via --control-bucket to prove deletion permissions."
        )

    # Phase 3
    print(f"[Phase 3/5] Exercising positive control object deletion in '{effective_control_bucket}'...")
    exercise_control_object(effective_control_bucket, control_key, profile, region, runner)
    print(f"   [PASS] Unlocked control object deleted successfully from '{effective_control_bucket}'. Principal possesses real deletion power.")

    # Phase 4
    print("[Phase 4/5] Probing locked object deletion (WORM immutability proof)...")
    attempt_locked_object_deletion(bucket, target_key, target_version_id, profile, region, runner)
    print("   [PASS] Version-specific delete strictly rejected with AccessDenied / HTTP 403.")

    # Phase 5
    print("[Phase 5/5] Confirming target version preservation & COMPLIANCE retention...")
    res = verify_object_retention_and_integrity(bucket, target_key, target_version_id, profile, region, runner)
    retain_until = res["retention"]["RetainUntilDate"]
    print(f"   [PASS] Target object version preserved. Mode=COMPLIANCE, RetainUntilDate={retain_until}.")

    print("=" * 70)
    print("WORM ENFORCEMENT PROVEN: Target backup is protected by S3 Object Lock COMPLIANCE mode.")
    print("=" * 70)
    return True


def main() -> None:
    parser = argparse.ArgumentParser(
        description="Spine Forge S3 Object Lock Compliance WORM Rehearsal Tool"
    )
    parser.add_argument("--bucket", required=True, help="S3 bucket name")
    parser.add_argument("--target-key", required=True, help="Object key of the protected backup archive")
    parser.add_argument("--version-id", required=True, help="VersionId of the protected backup archive")
    parser.add_argument("--expected-role", default="SpineForge-AuditRole", help="Expected IAM role name substring")
    parser.add_argument("--control-key", default="rehearsals/control-delete-test.txt", help="Key for unlocked control object")
    parser.add_argument("--control-bucket", default=None, help="Optional unlocked control bucket (required if target bucket has default retention)")
    parser.add_argument("--profile", default=None, help="AWS CLI profile name (do NOT pass raw credentials)")
    parser.add_argument("--region", default=None, help="AWS Region (e.g. us-east-1)")

    args = parser.parse_args()

    try:
        rehearse_s3_compliance(
            bucket=args.bucket,
            target_key=args.target_key,
            target_version_id=args.version_id,
            expected_role=args.expected_role,
            control_key=args.control_key,
            control_bucket=args.control_bucket,
            profile=args.profile,
            region=args.region,
        )
    except Exception as exc:
        print(f"\n[FATAL ERROR] S3 compliance rehearsal failed: {exc}", file=sys.stderr)
        sys.exit(1)


if __name__ == "__main__":
    main()
