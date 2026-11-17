# ==============================================================================
# Spine Forge — Clean Working Tree Verifier (PowerShell)
# Safely verifies working tree status across all Windows/Sandbox environments
# ==============================================================================
$ErrorActionPreference = "Stop"

Write-Host "Verifying Git working tree status (with safe.directory bypass)..."
$status = git -c safe.directory=* status --porcelain

if (-not $status) {
    Write-Host "WORKING TREE IS CLEAN. 0 uncommitted changes." -ForegroundColor Green
    git -c safe.directory=* log -n 1 --oneline
    exit 0
} else {
    Write-Error "WORKING TREE HAS UNCOMMITTED CHANGES:`n$status"
    exit 1
}
