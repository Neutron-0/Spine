#!/usr/bin/env python3
"""
Spine Forge - CI Workflow Gating Contract & Job Dependency Verification Test

Validates that .github/workflows/ci.yml correctly wires the event-aware change detector
(scripts/check_ci_change_scope.py) to gate heavy matrix jobs while leaving security/linting
jobs unconditionally active.

Verifies:
1. 'detect-changes' job outputs 'run_heavy_matrix' via check_ci_change_scope.py.
2. Heavy test jobs (rust-git-alternative, rust-github-backend, database-tests, frontend-checks)
   strictly require 'detect-changes' in their 'needs' and gate on 'run_heavy_matrix == 'true''.
3. Fast/security jobs (secret-scan, infrastructure-template, scripts-and-compose)
   are NEVER gated by 'detect-changes' (they always execute for rapid feedback).
4. End-to-end evaluation with ephemeral git fixtures confirms:
   - Push with code change -> heavy jobs execute.
   - Push with docs only -> heavy jobs skipped, security/scripts/cfn jobs execute.
   - Push with multi-commit (code + docs) -> heavy jobs execute.
   - Pull request with code change -> heavy jobs execute.
   - Pull request with docs only -> heavy jobs skipped.
   - Initial push (zero before-SHA) -> heavy jobs execute.
   - workflow_dispatch (manual trigger) -> heavy jobs execute.
"""

import os
import re
import shutil
import subprocess
import tempfile
import unittest
import yaml

from check_ci_change_scope import evaluate_scope

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
WORKFLOW_PATH = os.path.join(REPO_ROOT, ".github", "workflows", "ci.yml")

HEAVY_JOBS = {
    "rust-git-alternative",
    "rust-github-backend",
    "database-tests",
    "frontend-checks",
}

UNCONDITIONAL_JOBS = {
    "secret-scan",
    "infrastructure-template",
    "scripts-and-compose",
}


class TestCiWorkflowGatingContract(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        with open(WORKFLOW_PATH, "r", encoding="utf-8") as f:
            cls.workflow = yaml.safe_load(f)
        cls.jobs = cls.workflow.get("jobs", {})

    def test_workflow_detect_changes_job_structure(self):
        """Verify detect-changes job exists, outputs run_heavy_matrix, and calls check_ci_change_scope.py."""
        self.assertIn("detect-changes", self.jobs, "Missing 'detect-changes' job in ci.yml")
        detect_job = self.jobs["detect-changes"]
        outputs = detect_job.get("outputs", {})
        self.assertIn("run_heavy_matrix", outputs, "Missing 'run_heavy_matrix' output in detect-changes")

        # Find evaluate step
        steps = detect_job.get("steps", [])
        detector_step = None
        for step in steps:
            run_cmd = step.get("run", "")
            if "check_ci_change_scope.py" in run_cmd:
                detector_step = step
                break

        self.assertIsNotNone(detector_step, "detect-changes must execute check_ci_change_scope.py")
        cmd = detector_step.get("run", "")
        self.assertIn("--event-name", cmd)
        self.assertIn("--before", cmd)
        self.assertIn("--sha", cmd)
        self.assertIn("--pr-base", cmd)
        self.assertIn("--output-file", cmd)

    def test_heavy_jobs_strict_gating(self):
        """Verify all heavy jobs require detect-changes and gate on run_heavy_matrix == 'true'."""
        for job_id in HEAVY_JOBS:
            self.assertIn(job_id, self.jobs, f"Expected heavy job '{job_id}' in ci.yml")
            job = self.jobs[job_id]

            # Check 'needs'
            needs = job.get("needs", [])
            if isinstance(needs, str):
                needs = [needs]
            self.assertIn(
                "detect-changes",
                needs,
                f"Job '{job_id}' must include 'detect-changes' in its needs dependencies",
            )

            # Check 'if' condition
            condition = str(job.get("if", "")).strip()
            self.assertIn(
                "needs.detect-changes.outputs.run_heavy_matrix == 'true'",
                condition,
                f"Job '{job_id}' must strictly gate on 'needs.detect-changes.outputs.run_heavy_matrix == 'true'', found: '{condition}'",
            )

    def test_unconditional_jobs_not_gated(self):
        """Verify fast security and linting jobs are NEVER gated by detect-changes."""
        for job_id in UNCONDITIONAL_JOBS:
            self.assertIn(job_id, self.jobs, f"Expected fast job '{job_id}' in ci.yml")
            job = self.jobs[job_id]

            needs = job.get("needs", [])
            if isinstance(needs, str):
                needs = [needs]
            self.assertNotIn(
                "detect-changes",
                needs,
                f"Fast job '{job_id}' must NOT depend on 'detect-changes'",
            )

            condition = str(job.get("if", "")).strip()
            self.assertNotIn(
                "run_heavy_matrix",
                condition,
                f"Fast job '{job_id}' must NOT gate on run_heavy_matrix",
            )


class TestCiWorkflowSimulatedEvaluation(unittest.TestCase):
    """Simulates how GitHub Actions evaluates workflow conditions across real git event histories."""

    @classmethod
    def setUpClass(cls):
        cls.test_dir = tempfile.mkdtemp(prefix="spine_wf_contract_")
        cls.repo_dir = os.path.join(cls.test_dir, "repo")
        os.makedirs(cls.repo_dir)

        def git(args):
            res = subprocess.run(["git"] + args, cwd=cls.repo_dir, capture_output=True, text=True)
            if res.returncode != 0:
                raise RuntimeError(f"git {' '.join(args)} failed: {res.stderr}")
            return res.stdout.strip()

        cls.git = staticmethod(git)
        cls.git(["init", "-b", "main"])
        cls.git(["config", "user.name", "Test Runner"])
        cls.git(["config", "user.email", "runner@spine.build"])

        # Initial commit c1
        with open(os.path.join(cls.repo_dir, "README.md"), "w") as f:
            f.write("# Spine\n")
        cls.git(["add", "README.md"])
        cls.git(["commit", "-m", "init"])
        cls.c1 = cls.git(["rev-parse", "HEAD"])

        # Commit c2 (code): adds src/main.rs
        os.makedirs(os.path.join(cls.repo_dir, "src"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "src", "main.rs"), "w") as f:
            f.write("fn main() {}\n")
        cls.git(["add", "src/main.rs"])
        cls.git(["commit", "-m", "add code"])
        cls.c2 = cls.git(["rev-parse", "HEAD"])

        # Commit c3 (docs only): modifies docs/guide.md
        os.makedirs(os.path.join(cls.repo_dir, "docs"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "docs", "guide.md"), "w") as f:
            f.write("# Guide\n")
        cls.git(["add", "docs/guide.md"])
        cls.git(["commit", "-m", "add docs"])
        cls.c3 = cls.git(["rev-parse", "HEAD"])

    @classmethod
    def tearDownClass(cls):
        shutil.rmtree(cls.test_dir, ignore_errors=True)

    def evaluate_job_activation(self, run_heavy_matrix: bool):
        """Simulate GitHub Actions job activation table."""
        activated_jobs = set()
        for job in UNCONDITIONAL_JOBS:
            activated_jobs.add(job)
        if run_heavy_matrix:
            for job in HEAVY_JOBS:
                activated_jobs.add(job)
        return activated_jobs

    def test_push_code_activates_heavy_matrix(self):
        """A push touching code files must activate the full heavy matrix."""
        heavy, reason, _ = evaluate_scope("push", self.c2, before=self.c1, cwd=self.repo_dir)
        self.assertTrue(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(HEAVY_JOBS.issubset(active))

    def test_push_docs_only_skips_heavy_matrix_preserves_security(self):
        """A push touching only docs must skip heavy matrix while running security/linters."""
        heavy, reason, _ = evaluate_scope("push", self.c3, before=self.c2, cwd=self.repo_dir)
        self.assertFalse(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(UNCONDITIONAL_JOBS.issubset(active))
        for j in HEAVY_JOBS:
            self.assertNotIn(j, active)

    def test_multi_commit_push_code_then_docs_activates_heavy_matrix(self):
        """A push containing c1..c3 (code followed by docs) must NOT skip heavy matrix."""
        heavy, reason, _ = evaluate_scope("push", self.c3, before=self.c1, cwd=self.repo_dir)
        self.assertTrue(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(HEAVY_JOBS.issubset(active))

    def test_pull_request_code_activates_heavy_matrix(self):
        """PR from main to branch touching code must activate heavy matrix."""
        heavy, reason, _ = evaluate_scope("pull_request", self.c2, pr_base=self.c1, cwd=self.repo_dir)
        self.assertTrue(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(HEAVY_JOBS.issubset(active))

    def test_pull_request_docs_only_skips_heavy_matrix(self):
        """PR touching only docs must skip heavy matrix."""
        heavy, reason, _ = evaluate_scope("pull_request", self.c3, pr_base=self.c2, cwd=self.repo_dir)
        self.assertFalse(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(UNCONDITIONAL_JOBS.issubset(active))
        for j in HEAVY_JOBS:
            self.assertNotIn(j, active)

    def test_initial_push_activates_heavy_matrix(self):
        """Initial push with zero before-SHA must fail closed to heavy matrix."""
        heavy, reason, _ = evaluate_scope("push", self.c1, before="0" * 40, cwd=self.repo_dir)
        self.assertTrue(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(HEAVY_JOBS.issubset(active))

    def test_workflow_dispatch_activates_heavy_matrix(self):
        """Manual trigger must activate heavy matrix unconditionally."""
        heavy, reason, _ = evaluate_scope("workflow_dispatch", self.c3, cwd=self.repo_dir)
        self.assertTrue(heavy)
        active = self.evaluate_job_activation(heavy)
        self.assertTrue(HEAVY_JOBS.issubset(active))


if __name__ == "__main__":
    unittest.main()
