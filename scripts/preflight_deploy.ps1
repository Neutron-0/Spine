# ==============================================================================
# Spine Forge - Production Deployment Preflight Validator (PowerShell)
# Validates environment secrets, TLS certificate readability, and Compose syntax
# ==============================================================================

param(
    [string]$EnvFile = ".env",
    [string]$ComposeFile = "docker-compose.prod.yml"
)

$ErrorActionPreference = "Stop"

Write-Host "=================================================================="
Write-Host " Spine Forge - Production Deployment Preflight Check"
Write-Host "=================================================================="
Write-Host "Target Environment File: $EnvFile"
Write-Host "Target Compose File:     $ComposeFile"
Write-Host "------------------------------------------------------------------"

$Failed = $false

# 1. Verify environment file exists
if (-not (Test-Path $EnvFile -PathType Leaf)) {
    Write-Error "[FAIL] Environment configuration file '$EnvFile' not found."
    Write-Host "       Copy .env.example to .env and configure production secrets."
    exit 1
}

Write-Host "[PASS] Environment configuration file exists: $EnvFile"

# Parse environment file
$EnvVars = @{}
Get-Content $EnvFile | ForEach-Object {
    $line = $_.Trim()
    if ($line -and -not $line.StartsWith("#")) {
        $parts = $line.Split("=", 2)
        if ($parts.Length -eq 2) {
            $key = $parts[0].Trim()
            $val = $parts[1].Trim()
            # Strip quotes if present
            if (($val.StartsWith('"') -and $val.EndsWith('"')) -or ($val.StartsWith("'") -and $val.EndsWith("'"))) {
                $val = $val.Substring(1, $val.Length - 2)
            }
            $EnvVars[$key] = $val
        }
    }
}

function Check-Secret([string]$VarName, [int]$MinLen) {
    if (-not $EnvVars.ContainsKey($VarName) -or [string]::IsNullOrWhiteSpace($EnvVars[$VarName])) {
        Write-Host "[FAIL] Required secret '$VarName' is unset or empty." -ForegroundColor Red
        $script:Failed = $true
    } elseif ($EnvVars[$VarName].Length -lt $MinLen) {
        Write-Host "[FAIL] Secret '$VarName' is too short ($($EnvVars[$VarName].Length) chars; minimum $MinLen required)." -ForegroundColor Red
        $script:Failed = $true
    } else {
        Write-Host "[PASS] Secret '$VarName' meets minimum length ($MinLen+ chars)." -ForegroundColor Green
    }
}

Check-Secret "POSTGRES_ADMIN_PASSWORD" 16
Check-Secret "POSTGRES_APP_PASSWORD" 16
Check-Secret "POSTGRES_MIGRATOR_PASSWORD" 16
Check-Secret "SPINE_AUTH_SECRET_KEY" 32
Check-Secret "SPINE_LFS_SECRET" 32

# Verify backup encryption passphrase (either raw secret or secure mounted file)
$BackupPass = $EnvVars["SPINE_BACKUP_PASSPHRASE"]
$BackupPassFile = $EnvVars["SPINE_BACKUP_PASSPHRASE_FILE"]

if (-not [string]::IsNullOrWhiteSpace($BackupPass)) {
    Check-Secret "SPINE_BACKUP_PASSPHRASE" 16
} elseif (-not [string]::IsNullOrWhiteSpace($BackupPassFile)) {
    if (Test-Path $BackupPassFile -PathType Leaf) {
        Write-Host "[PASS] Backup passphrase file verified readable: $BackupPassFile" -ForegroundColor Green
    } else {
        Write-Host "[FAIL] Backup passphrase file '$BackupPassFile' not found." -ForegroundColor Red
        $Failed = $true
    }
} else {
    Write-Host "[FAIL] Either SPINE_BACKUP_PASSPHRASE or SPINE_BACKUP_PASSPHRASE_FILE must be configured." -ForegroundColor Red
    $Failed = $true
}

# Check TLS certificates
$CertFile = $EnvVars["SPINE_TLS_CERT_FILE"]
$KeyFile = $EnvVars["SPINE_TLS_KEY_FILE"]

if ([string]::IsNullOrWhiteSpace($CertFile)) {
    Write-Host "[FAIL] SPINE_TLS_CERT_FILE is unset in $EnvFile." -ForegroundColor Red
    $Failed = $true
} elseif (-not (Test-Path $CertFile -PathType Leaf)) {
    Write-Host "[FAIL] TLS certificate file '$CertFile' does not exist." -ForegroundColor Red
    $Failed = $true
} elseif ((Get-Item $CertFile).Length -eq 0) {
    Write-Host "[FAIL] TLS certificate file '$CertFile' is empty (0 bytes)." -ForegroundColor Red
    $Failed = $true
} else {
    Write-Host "[PASS] TLS certificate file verified readable: $CertFile" -ForegroundColor Green
}

if ([string]::IsNullOrWhiteSpace($KeyFile)) {
    Write-Host "[FAIL] SPINE_TLS_KEY_FILE is unset in $EnvFile." -ForegroundColor Red
    $Failed = $true
} elseif (-not (Test-Path $KeyFile -PathType Leaf)) {
    Write-Host "[FAIL] TLS private key file '$KeyFile' does not exist." -ForegroundColor Red
    $Failed = $true
} elseif ((Get-Item $KeyFile).Length -eq 0) {
    Write-Host "[FAIL] TLS private key file '$KeyFile' is empty (0 bytes)." -ForegroundColor Red
    $Failed = $true
} else {
    Write-Host "[PASS] TLS private key file verified readable: $KeyFile" -ForegroundColor Green
}

# Check Compose configuration
if (Get-Command "docker" -ErrorAction SilentlyContinue) {
    Write-Host "Validating Docker Compose interpolation..."
    & docker compose --env-file $EnvFile -f $ComposeFile config --quiet 2>$null
    if ($LASTEXITCODE -eq 0) {
        Write-Host "[PASS] Docker Compose configuration interpolated cleanly." -ForegroundColor Green
    } else {
        Write-Host "[FAIL] Docker Compose configuration validation failed for '$ComposeFile'." -ForegroundColor Red
        & docker compose --env-file $EnvFile -f $ComposeFile config
        $Failed = $true
    }
} else {
    Write-Host "[WARN] Docker Compose CLI not detected; skipping compose config validation." -ForegroundColor Yellow
}

Write-Host "------------------------------------------------------------------"
if ($Failed) {
    Write-Host "Preflight check FAILED. Correct configuration errors above before deploying." -ForegroundColor Red
    exit 1
}

Write-Host "Preflight check PASSED. Cluster configuration is ready for deployment." -ForegroundColor Green
exit 0
