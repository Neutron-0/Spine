#!/usr/bin/env python3
"""
Test Suite: Legacy LFS Migration Preflight & Operator Remediation Safety.
Verifies:
1. Read-only inventory flags invalid OIDs, missing repositories, ambiguous ownership, duplicate collisions, and non-LFS exclusions.
2. Operator remediation requires explicit mappings; never infers ownership.
3. Malformed input (missing fields, blank strings, empty lists, duplicate inputs) fails closed.
4. Non-LFS manifests (release assets, packages, artifact versions) cannot be remediated.
5. Missing repositories in forge_repositories cannot be assigned.
6. Duplicate collisions for (repo, oid) across distinct manifests fail closed.
7. Idempotent rerun recognizes already-correct rows.
"""

import sys
from pathlib import Path

# Add parent directory to sys.path
sys.path.insert(0, str(Path(__file__).resolve().parent))

from preflight_lfs_migration import inventory_manifests, validate_operator_mappings


def test_preflight_inventory():
    known_repos = {"repo_1", "repo_2"}
    non_lfs_ids = {"man_release_1", "man_pkg_1"}
    mapped_ids = {"man_already_mapped"}

    manifests = [
        # 1. Non-LFS release asset
        {"id": "man_release_1", "artifact_id": "a" * 64, "repository_id": "repo_1"},
        # 2. Already mapped LFS
        {"id": "man_already_mapped", "artifact_id": "b" * 64, "repository_id": "repo_1"},
        # 3. Valid ready LFS
        {"id": "man_valid", "artifact_id": "c" * 64, "repository_id": "repo_1"},
        # 4. Invalid OID
        {"id": "man_invalid_oid", "artifact_id": "not-64-hex", "repository_id": "repo_1"},
        # 5. Missing repo
        {"id": "man_missing_repo", "artifact_id": "d" * 64, "repository_id": "repo_deleted"},
        # 6. Ambiguous ownership (lacks repo)
        {"id": "man_ambiguous", "artifact_id": "e" * 64},
        # 7. Collision (2 manifests same repo and OID)
        {"id": "man_col_1", "artifact_id": "f" * 64, "repository_id": "repo_2"},
        {"id": "man_col_2", "artifact_id": "f" * 64, "repository_id": "repo_2"},
    ]

    inv = inventory_manifests(manifests, known_repos, non_lfs_ids, mapped_ids)

    assert inv["total_manifests_inspected"] == 8
    assert inv["non_lfs_excluded_count"] == 1
    assert inv["already_mapped_count"] == 1
    assert inv["ready_to_migrate_count"] == 1  # only man_valid
    assert len(inv["invalid_oids"]) == 1
    assert inv["invalid_oids"][0]["manifest_id"] == "man_invalid_oid"
    assert len(inv["missing_repositories"]) == 1
    assert inv["missing_repositories"][0]["target_repo_id"] == "repo_deleted"
    assert len(inv["ambiguous_ownership"]) == 1
    assert inv["ambiguous_ownership"][0]["manifest_id"] == "man_ambiguous"
    assert len(inv["duplicate_collisions"]) == 1
    assert inv["duplicate_collisions"][0]["repository_id"] == "repo_2"
    assert not inv["is_safe_to_migrate"]
    print("  [PASS] Preflight inventory accurately categorizes invalid OIDs, missing repos, ambiguous ownership, duplicate collisions, and non-LFS exclusions.")


def test_operator_remediation_safety():
    known_repos = {"repo_1", "repo_2"}
    non_lfs_ids = {"man_release_1"}
    manifests = {
        "man_ambiguous_1": {"artifact_id": "1" * 64},
        "man_ambiguous_2": {"artifact_id": "2" * 64},
        "man_invalid_oid": {"artifact_id": "bad-oid"},
        "man_release_1": {"artifact_id": "3" * 64},
        "man_already_done": {"artifact_id": "4" * 64, "repository_id": "repo_1"},
    }

    # 1. Empty mappings fail closed
    try:
        validate_operator_mappings([], manifests, known_repos, non_lfs_ids)
        assert False, "Empty mappings must fail"
    except ValueError as e:
        assert "empty" in str(e).lower()
    print("  [PASS] Empty operator mapping list rejected.")

    # 2. Duplicate manifest ID in input fails closed
    try:
        validate_operator_mappings(
            [
                {"manifest_id": "man_ambiguous_1", "repository_id": "repo_1"},
                {"manifest_id": "man_ambiguous_1", "repository_id": "repo_2"},
            ],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Duplicate manifest_id in input must fail"
    except ValueError as e:
        assert "duplicate" in str(e).lower()
    print("  [PASS] Duplicate manifest ID in input rejected.")

    # 3. Non-existent manifest fails closed
    try:
        validate_operator_mappings(
            [{"manifest_id": "man_unknown", "repository_id": "repo_1"}],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Non-existent manifest must fail"
    except ValueError as e:
        assert "does not exist" in str(e)
    print("  [PASS] Non-existent manifest mapping rejected.")

    # 4. Non-LFS manifest fails closed
    try:
        validate_operator_mappings(
            [{"manifest_id": "man_release_1", "repository_id": "repo_1"}],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Non-LFS manifest must fail"
    except ValueError as e:
        assert "non-lfs" in str(e).lower()
    print("  [PASS] Non-LFS manifest remediation rejected.")

    # 5. Invalid OID fails closed
    try:
        validate_operator_mappings(
            [{"manifest_id": "man_invalid_oid", "repository_id": "repo_1"}],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Invalid OID must fail"
    except ValueError as e:
        assert "invalid artifact_id" in str(e).lower()
    print("  [PASS] Invalid OID manifest remediation rejected.")

    # 6. Target repo not in forge_repositories fails closed
    try:
        validate_operator_mappings(
            [{"manifest_id": "man_ambiguous_1", "repository_id": "repo_nonexistent"}],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Missing repo must fail"
    except ValueError as e:
        assert "does not exist in forge_repositories" in str(e)
    print("  [PASS] Non-existent target repository rejected.")

    # 7. Collision between two mappings for same (repo, oid) fails closed
    manifests["man_ambiguous_dup"] = {"artifact_id": "1" * 64}
    try:
        validate_operator_mappings(
            [
                {"manifest_id": "man_ambiguous_1", "repository_id": "repo_1"},
                {"manifest_id": "man_ambiguous_dup", "repository_id": "repo_1"},
            ],
            manifests,
            known_repos,
            non_lfs_ids,
        )
        assert False, "Duplicate collision must fail"
    except ValueError as e:
        assert "duplicate collision" in str(e).lower()
    print("  [PASS] Duplicate collision across mappings rejected.")

    # 8. Valid remediation succeeds and tracks already-correct idempotent rerun
    res = validate_operator_mappings(
        [
            {"manifest_id": "man_ambiguous_1", "repository_id": "repo_1"},
            {"manifest_id": "man_ambiguous_2", "repository_id": "repo_2"},
            {"manifest_id": "man_already_done", "repository_id": "repo_1"},
        ],
        manifests,
        known_repos,
        non_lfs_ids,
    )
    assert res["mappings_validated"] == 3
    assert res["manifests_remediated"] == 2
    assert res["already_correct"] == 1
    print("  [PASS] Valid remediation succeeds with idempotent rerun tracking.")


if __name__ == "__main__":
    test_preflight_inventory()
    test_operator_remediation_safety()
    print("\n[SUCCESS] All preflight & remediation safety tests passed successfully!")
