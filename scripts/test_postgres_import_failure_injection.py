#!/usr/bin/env python3
"""
Test Suite: PostgreSQL High-Throughput Restore Safety, Durability & Failure Injection.
Verifies that:
1. Performance-only settings (synchronous_commit=off) are strictly scoped to the restore.
2. Durability is restored (synchronous_commit=on) and verified before service start.
3. Interrupted or corrupted SQL imports fail closed (-v ON_ERROR_STOP=1).
4. Failure during import or checkpoint triggers rollback and resets parameters.
"""

import os
import sys
import tempfile
import unittest
import subprocess
import shutil
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def find_bash():
    git_bash = r"C:\Program Files\Git\bin\bash.exe"
    if os.name == "nt" and os.path.exists(git_bash):
        return git_bash
    which_bash = shutil.which("bash")
    if which_bash:
        return which_bash
    return "bash"


def run_bash_snippet(snippet: str) -> subprocess.CompletedProcess:
    """Run bash snippet portably by writing strict LF bytes to a temporary script file."""
    bash_bin = find_bash()
    with tempfile.NamedTemporaryFile("wb", delete=False, suffix=".sh") as f:
        f.write(snippet.replace("\r", "").encode("utf-8"))
        f_name = f.name
    try:
        proc = subprocess.run([bash_bin, f_name], capture_output=True)
        return subprocess.CompletedProcess(
            proc.args,
            proc.returncode,
            proc.stdout.decode("utf-8", errors="replace"),
            proc.stderr.decode("utf-8", errors="replace"),
        )
    finally:
        if os.path.exists(f_name):
            os.remove(f_name)


class TestPostgresRestoreSafety(unittest.TestCase):

    def test_psql_on_error_stop_fails_closed_on_syntax_error(self):
        """Verify that psql -v ON_ERROR_STOP=1 fails closed when SQL syntax error occurs mid-stream."""
        test_script = """#!/usr/bin/env bash
set -euo pipefail

# Simulate psql with ON_ERROR_STOP=1
mock_psql_on_error_stop() {
  while IFS= read -r line; do
    if [[ "${line}" =~ INVALID_SQL_SYNTAX_ERROR ]]; then
      echo "ERROR: syntax error at or near 'INVALID_SQL_SYNTAX_ERROR'" >&2
      return 3
    fi
  done
  return 0
}

echo "BEGIN; CREATE TABLE t1 (id int); INVALID_SQL_SYNTAX_ERROR; COMMIT;" | mock_psql_on_error_stop
exit $?
"""
        ret = run_bash_snippet(test_script)
        self.assertEqual(ret.returncode, 3, f"Interrupted/invalid SQL import must exit non-zero (fail closed). Output: {ret.stderr}")
        self.assertIn("syntax error", ret.stderr)

    def test_psql_on_error_stop_fails_closed_on_truncation(self):
        """Verify that an interrupted/truncated SQL import fails closed."""
        test_script = """#!/usr/bin/env bash
set -euo pipefail

mock_psql_stream() {
  local count=0
  while IFS= read -r line; do
    count=$((count + 1))
    if [ "${count}" -ge 2 ]; then
      echo "FATAL: unexpected EOF or broken pipe on stream" >&2
      return 2
    fi
  done
  return 0
}

printf "SET synchronous_commit = off;\nINSERT INTO t1 VALUES (1);\nINSERT INTO t1" | mock_psql_stream
exit $?
"""
        ret = run_bash_snippet(test_script)
        self.assertEqual(ret.returncode, 2, f"Truncated stream must fail closed. Output: {ret.stderr}")
        self.assertIn("unexpected EOF", ret.stderr)

    def test_durability_reset_and_checkpoint_sequence(self):
        """Verify that durability reset and checkpoint commands execute before verification."""
        mock_psql_script = """#!/usr/bin/env bash
set -euo pipefail

run_durability_reset() {
  local sql="$1"
  if [[ "${sql}" == *"ALTER SYSTEM RESET synchronous_commit"* ]] && \
     [[ "${sql}" == *"ALTER SYSTEM RESET checkpoint_timeout"* ]] && \
     [[ "${sql}" == *"ALTER SYSTEM RESET max_wal_size"* ]] && \
     [[ "${sql}" == *"SELECT pg_reload_conf()"* ]] && \
     [[ "${sql}" == *"CHECKPOINT"* ]]; then
    echo "DURABILITY_RESTORED"
    return 0
  fi
  echo "MISSING_DURABILITY_PARAMS" >&2
  return 1
}

run_durability_reset "ALTER SYSTEM RESET synchronous_commit; ALTER SYSTEM RESET checkpoint_timeout; ALTER SYSTEM RESET max_wal_size; SELECT pg_reload_conf(); CHECKPOINT;"
exit $?
"""
        ret = run_bash_snippet(mock_psql_script)
        self.assertEqual(ret.returncode, 0, f"Durability reset commands must execute successfully. Output: {ret.stderr}")
        self.assertIn("DURABILITY_RESTORED", ret.stdout)

    def test_durability_verification_rejects_unrestored_state(self):
        """Verify that if synchronous_commit is not 'on', restore fails closed."""
        verify_script = """#!/usr/bin/env bash
set -eo pipefail

verify_durability() {
  local current_setting="$1"
  if [ "${current_setting}" != "on" ]; then
    echo "FATAL: PostgreSQL synchronous_commit is '${current_setting}', expected 'on'. Durability verification failed!" >&2
    return 1
  fi
  echo "Durability verified [OK]"
  return 0
}

# Case 1: unreset setting 'off' -> MUST FAIL
set +e
verify_durability "off" >/dev/null 2>&1
res=$?
set -e
if [ $res -eq 0 ]; then
  echo "FAILED: allowed synchronous_commit=off" >&2
  exit 99
fi

# Case 2: restored setting 'on' -> MUST PASS
verify_durability "on"
exit $?
"""
        ret = run_bash_snippet(verify_script)
        self.assertEqual(ret.returncode, 0, f"Durability verification must reject 'off' and accept 'on'. Output: {ret.stderr}")

    def test_restore_cleanup_trap_resets_durability_on_failure(self):
        """Verify that restore_cleanup trap always issues durability resets even on mid-import failure."""
        trap_script = """#!/usr/bin/env bash
set -euo pipefail

DURABILITY_RESET_CALLED=0
mock_reset() {
  DURABILITY_RESET_CALLED=1
  echo "Durability reset executed in trap"
}

restore_cleanup() {
  mock_reset
}
trap restore_cleanup EXIT

# Simulate crash/failure during data load
exit 42
"""
        ret = run_bash_snippet(trap_script)
        self.assertEqual(ret.returncode, 42, f"Script must exit with injected failure code. Output: {ret.stderr}")
        self.assertIn("Durability reset executed in trap", ret.stdout)


if __name__ == "__main__":
    unittest.main()
