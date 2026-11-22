#!/usr/bin/env python3
"""
Unit and Integration Tests for Event-Aware CI Change Scope Detector
(scripts/check_ci_change_scope.py).
"""

import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import check_ci_change_scope


class TestCiChangeScope(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        # Create an ephemeral git repository with committed fixtures
        cls.temp_dir = tempfile.TemporaryDirectory()
        cls.repo_dir = cls.temp_dir.name

        def git(args):
            subprocess.run(["git"] + args, cwd=cls.repo_dir, check=True, capture_output=True)

        git(["init", "-b", "main"])
        git(["config", "user.name", "CI Test"])
        git(["config", "user.email", "ci-test@example.com"])

        # Commit 0: Initial commit
        with open(os.path.join(cls.repo_dir, "README.md"), "w") as f:
            f.write("# Init\n")
        git(["add", "README.md"])
        git(["commit", "-m", "Initial commit"])
        cls.c0 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Commit 1: Docs only
        os.makedirs(os.path.join(cls.repo_dir, "docs"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "docs", "guide.md"), "w") as f:
            f.write("# Guide\n")
        git(["add", "docs/guide.md"])
        git(["commit", "-m", "docs: add guide"])
        cls.c1 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Commit 2: Code change
        os.makedirs(os.path.join(cls.repo_dir, "src"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "src", "main.rs"), "w") as f:
            f.write("fn main() {}\n")
        git(["add", "src/main.rs"])
        git(["commit", "-m", "feat: add main.rs"])
        cls.c2 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Commit 3: Docs change after code change
        with open(os.path.join(cls.repo_dir, "docs", "api.md"), "w") as f:
            f.write("# API\n")
        git(["add", "docs/api.md"])
        git(["commit", "-m", "docs: add api.md"])
        cls.c3 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Commit 4: Mixed commit (code + doc)
        with open(os.path.join(cls.repo_dir, "src", "lib.rs"), "w") as f:
            f.write("pub fn run() {}\n")
        with open(os.path.join(cls.repo_dir, "docs", "lib.md"), "w") as f:
            f.write("# Lib\n")
        git(["add", "src/lib.rs", "docs/lib.md"])
        git(["commit", "-m", "feat+docs: mixed commit"])
        cls.c4 = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Branch for PR tests
        git(["checkout", "-b", "pr-branch", cls.c1])
        os.makedirs(os.path.join(cls.repo_dir, "src"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "src", "pr_feature.rs"), "w") as f:
            f.write("// pr feature\n")
        git(["add", "src/pr_feature.rs"])
        git(["commit", "-m", "feat: pr code feature"])
        cls.c_pr_code = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        git(["checkout", "-b", "pr-docs-only", cls.c1])
        os.makedirs(os.path.join(cls.repo_dir, "docs"), exist_ok=True)
        with open(os.path.join(cls.repo_dir, "docs", "pr_doc.md"), "w") as f:
            f.write("# PR doc\n")
        git(["add", "docs/pr_doc.md"])
        git(["commit", "-m", "docs: pr doc only"])
        cls.c_pr_doc = subprocess.run(["git", "rev-parse", "HEAD"], cwd=cls.repo_dir, capture_output=True, text=True).stdout.strip()

        # Checkout main
        git(["checkout", "main"])

    @classmethod
    def tearDownClass(cls):
        cls.temp_dir.cleanup()

    def test_docs_only_single_commit(self):
        """Docs-only commit in push range allows skipping heavy matrix."""
        run_heavy, reason, files = check_ci_change_scope.evaluate_scope(
            event_name="push",
            before=self.c0,
            sha=self.c1,
            cwd=self.repo_dir,
        )
        self.assertFalse(run_heavy)
        self.assertEqual(files, ["docs/guide.md"])
        self.assertIn("Skipping heavy matrix", reason)

    def test_code_commit_followed_by_docs_commit_in_one_push(self):
        """Push containing code commit followed by docs commit triggers full matrix."""
        # Range is c1..c3 (includes c2 code commit and c3 docs commit)
        run_heavy, reason, files = check_ci_change_scope.evaluate_scope(
            event_name="push",
            before=self.c1,
            sha=self.c3,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("src/main.rs", files)
        self.assertIn("docs/api.md", files)
        self.assertIn("Non-documentation files modified", reason)

    def test_mixed_commit_in_push(self):
        """Single push modifying both code and docs triggers full matrix."""
        # Range is c3..c4
        run_heavy, reason, files = check_ci_change_scope.evaluate_scope(
            event_name="push",
            before=self.c3,
            sha=self.c4,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("src/lib.rs", files)
        self.assertIn("Non-documentation files modified", reason)

    def test_pull_request_containing_code_changes(self):
        """Pull request modifying code triggers full matrix."""
        run_heavy, reason, files = check_ci_change_scope.evaluate_scope(
            event_name="pull_request",
            pr_base=self.c1,
            sha=self.c_pr_code,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("src/pr_feature.rs", files)
        self.assertIn("Non-documentation files modified", reason)

    def test_pull_request_containing_only_docs(self):
        """Pull request modifying only docs allows skipping heavy matrix."""
        run_heavy, reason, files = check_ci_change_scope.evaluate_scope(
            event_name="pull_request",
            pr_base=self.c1,
            sha=self.c_pr_doc,
            cwd=self.repo_dir,
        )
        self.assertFalse(run_heavy)
        self.assertEqual(files, ["docs/pr_doc.md"])
        self.assertIn("Skipping heavy matrix", reason)

    def test_initial_push_zero_before_sha(self):
        """Initial push with zero before-SHA fails closed to full matrix."""
        run_heavy, reason, _ = check_ci_change_scope.evaluate_scope(
            event_name="push",
            before="0" * 40,
            sha=self.c1,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("zero before-SHA", reason)

    def test_workflow_dispatch_forces_full_matrix(self):
        """Manual runs or workflow_dispatch events force full matrix."""
        run_heavy, reason, _ = check_ci_change_scope.evaluate_scope(
            event_name="workflow_dispatch",
            sha=self.c1,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("requires full matrix validation", reason)

    def test_unknown_commit_sha_fails_closed(self):
        """Unresolvable commit SHA (e.g. shallow clone) fails closed to full matrix."""
        run_heavy, reason, _ = check_ci_change_scope.evaluate_scope(
            event_name="push",
            before="1234567890abcdef1234567890abcdef12345678",
            sha=self.c1,
            cwd=self.repo_dir,
        )
        self.assertTrue(run_heavy)
        self.assertIn("shallow clone or missing commits", reason)


if __name__ == "__main__":
    unittest.main()
