#!/usr/bin/env python3
"""
Spine Forge - Event-Aware CI Change Scope & Path Detector

Evaluates whether a CI run can safely skip the heavy Rust & PostgreSQL test matrix
by analyzing the exact git change range corresponding to the triggering GitHub event.

Fail-Closed Rules:
1. On 'pull_request': diffs PR base SHA to PR head SHA.
2. On 'push': diffs 'before' SHA to target SHA.
   - Initial push (before = '0000000000000000000000000000000000000000') forces full matrix.
   - Unresolvable SHA or shallow clone boundary forces full matrix.
3. On 'workflow_dispatch', 'schedule', or unknown event: forces full matrix.
4. If ANY file outside 'docs/' and '*.md' was modified, added, deleted, or renamed: forces full matrix.
5. Only if 100% of changed files match docs/ or *.md does it emit run_heavy_matrix=false.
"""

import argparse
import os
import re
import subprocess
import sys
from typing import List, Optional, Tuple

ZERO_SHA = "0" * 40
DOCS_PATTERN = re.compile(r"^(docs/|.*\.md$)", re.IGNORECASE)


def run_git_command(args: List[str], cwd: Optional[str] = None) -> Tuple[int, str, str]:
    """Execute a git command in the target directory."""
    res = subprocess.run(
        ["git"] + args,
        cwd=cwd,
        capture_output=True,
        text=True,
    )
    return res.returncode, res.stdout.strip(), res.stderr.strip()


def resolve_changed_files_for_range(
    base_sha: str,
    head_sha: str,
    cwd: Optional[str] = None,
) -> Optional[List[str]]:
    """Get list of files modified between base_sha and head_sha."""
    # Verify both SHAs exist in local repository
    code1, _, _ = run_git_command(["cat-file", "-e", f"{base_sha}^{{commit}}"], cwd)
    code2, _, _ = run_git_command(["cat-file", "-e", f"{head_sha}^{{commit}}"], cwd)
    if code1 != 0 or code2 != 0:
        return None

    code, stdout, stderr = run_git_command(
        ["diff", "--name-only", f"{base_sha}..{head_sha}"],
        cwd,
    )
    if code != 0:
        return None

    files = [f.strip() for f in stdout.splitlines() if f.strip()]
    return files


def evaluate_scope(
    event_name: str,
    sha: str,
    before: Optional[str] = None,
    pr_base: Optional[str] = None,
    cwd: Optional[str] = None,
) -> Tuple[bool, str, List[str]]:
    """
    Returns:
      (run_heavy_matrix: bool, reason: str, changed_files: List[str])
    """
    if not event_name:
        return True, "Missing event_name. Failing closed to full matrix.", []

    event_name = event_name.strip().lower()

    if event_name == "pull_request":
        if not pr_base or pr_base.strip() == ZERO_SHA:
            return True, "Pull request base SHA is missing or zero. Failing closed to full matrix.", []
        changed = resolve_changed_files_for_range(pr_base.strip(), sha.strip(), cwd)
        if changed is None:
            return True, f"Unable to diff PR range {pr_base}..{sha}. Failing closed to full matrix.", []
        files = changed

    elif event_name == "push":
        if not before or before.strip() == ZERO_SHA:
            return True, "Initial push or new branch (zero before-SHA). Failing closed to full matrix.", []
        changed = resolve_changed_files_for_range(before.strip(), sha.strip(), cwd)
        if changed is None:
            return True, f"Unable to diff push range {before}..{sha} (shallow clone or missing commits). Failing closed to full matrix.", []
        files = changed

    else:
        return True, f"Event '{event_name}' requires full matrix validation by policy.", []

    if not files:
        return True, "Zero changed files detected in range. Failing closed to full matrix.", []

    non_docs = [f for f in files if not DOCS_PATTERN.match(f)]
    if non_docs:
        sample = non_docs[:3]
        return (
            True,
            f"Non-documentation files modified ({len(non_docs)} total, e.g. {sample}). Full matrix required.",
            files,
        )

    return (
        False,
        f"All {len(files)} changed file(s) are documentation/markdown. Skipping heavy matrix.",
        files,
    )


def main() -> None:
    parser = argparse.ArgumentParser(description="Evaluate CI change scope")
    parser.add_argument("--event-name", default=os.environ.get("GITHUB_EVENT_NAME", "push"))
    parser.add_argument("--sha", default=os.environ.get("GITHUB_SHA", "HEAD"))
    parser.add_argument("--before", default=None)
    parser.add_argument("--pr-base", default=None)
    parser.add_argument("--output-file", default=None)
    parser.add_argument("--git-dir", default=None)

    args = parser.parse_args()

    run_heavy, reason, changed = evaluate_scope(
        event_name=args.event_name,
        sha=args.sha,
        before=args.before,
        pr_base=args.pr_base,
        cwd=args.git_dir,
    )

    print("=" * 70)
    print("Spine Forge - CI Change Scope Evaluation")
    print("=" * 70)
    print(f"Event:            {args.event_name}")
    print(f"Commit SHA:       {args.sha}")
    print(f"Before SHA:       {args.before or '[none]'}")
    print(f"PR Base SHA:      {args.pr_base or '[none]'}")
    print(f"Total Files:      {len(changed)}")
    print(f"Decision:         run_heavy_matrix={'true' if run_heavy else 'false'}")
    print(f"Reason:           {reason}")
    print("=" * 70)

    val = "true" if run_heavy else "false"
    if args.output_file:
        with open(args.output_file, "a", encoding="utf-8") as f:
            f.write(f"run_heavy_matrix={val}\n")
    else:
        # Also write to GITHUB_OUTPUT if present
        gh_out = os.environ.get("GITHUB_OUTPUT")
        if gh_out and os.path.isfile(gh_out):
            with open(gh_out, "a", encoding="utf-8") as f:
                f.write(f"run_heavy_matrix={val}\n")


if __name__ == "__main__":
    main()
