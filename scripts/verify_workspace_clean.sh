#!/usr/bin/env bash
# ==============================================================================
# Spine Forge — Clean Working Tree Verifier
# Safely verifies working tree status across all Docker/CI/Sandbox environments
# ==============================================================================
set -euo pipefail

echo "Verifying Git working tree status (with safe.directory bypass)..."
STATUS=$(git -c safe.directory=* status --porcelain)

if [ -z "${STATUS}" ]; then
  echo "WORKING TREE IS CLEAN. 0 uncommitted changes."
  git -c safe.directory=* log -n 1 --oneline
  exit 0
else
  echo "WORKING TREE HAS UNCOMMITTED CHANGES:" >&2
  echo "${STATUS}" >&2
  exit 1
fi
