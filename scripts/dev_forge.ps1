<#
.SYNOPSIS
    Starts, stops, or checks status of a local Spine Forge development environment.
    - Spine Coordinator (optional, 127.0.0.1:8400)
    - Spine Forge Backend (127.0.0.1:3000)
    - Spine Forge Frontend (127.0.0.1:5173)
.EXAMPLE
    .\scripts\dev_forge.ps1 -Action start -InMemory
    .\scripts\dev_forge.ps1 -Action stop
    .\scripts\dev_forge.ps1 -Action status
    .\scripts\dev_forge.ps1 -Action clean
#>
param (
    [ValidateSet("start", "stop", "status", "clean")]
    [string]$Action = "start",
    [switch]$InMemory = $true,
    [switch]$StartFrontend = $true,
    [string]$CoordinatorAddr = "127.0.0.1:8400",
    [int]$Port = 3000
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$DevDir = Join-Path $RepoRoot ".spine-dev-forge"
$PidFile = Join-Path $DevDir "pids.json"
$StorageRoot = Join-Path $DevDir "storage"

function Clean-Forge {
    Write-Host "Cleaning Spine Forge development processes and state..." -ForegroundColor Yellow
    Stop-Forge
    if (Test-Path $DevDir) {
        Write-Host "Removing $DevDir..." -ForegroundColor Yellow
        Remove-Item -Recurse -Force $DevDir
    }
    Write-Host "Clean complete." -ForegroundColor Green
}

function Stop-Forge {
    if (Test-Path $PidFile) {
        $pids = Get-Content $PidFile | ConvertFrom-Json
        foreach ($prop in $pids.PSObject.Properties) {
            $procId = $prop.Value
            $procName = $prop.Name
            try {
                $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
                if ($p) {
                    Write-Host "Stopping $procName (PID $procId)..."
                    Stop-Process -Id $procId -Force
                }
            } catch {
                # Ignore process not found
            }
        }
        Remove-Item -Force $PidFile
    }

    # Fallback cleanup by image name
    Get-Process -Name "spine-api" -ErrorAction SilentlyContinue | Stop-Process -Force
    Write-Host "Spine Forge processes stopped." -ForegroundColor Green
}

function Get-ForgeStatus {
    if (-not (Test-Path $PidFile)) {
        Write-Host "No active Spine Forge instance registered in $PidFile." -ForegroundColor Yellow
        return
    }

    $pids = Get-Content $PidFile | ConvertFrom-Json
    Write-Host "=== Spine Forge Status ===" -ForegroundColor Cyan
    foreach ($prop in $pids.PSObject.Properties) {
        $procId = $prop.Value
        $procName = $prop.Name
        $p = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if ($p) {
            Write-Host "  [$procName]: Running (PID: $procId, Memory: $([math]::Round($p.WorkingSet64 / 1MB, 2)) MB)" -ForegroundColor Green
        } else {
            Write-Host "  [$procName]: Dead / Not Found (PID: $procId)" -ForegroundColor Red
        }
    }
}

function Start-Forge {
    Write-Host "Initializing Spine Forge local environment..." -ForegroundColor Cyan

    if (-not (Test-Path $DevDir)) {
        New-Item -ItemType Directory -Path $DevDir | Out-Null
    }
    if (-not (Test-Path $StorageRoot)) {
        New-Item -ItemType Directory -Path $StorageRoot | Out-Null
    }

    # Check or compile backend
    $BackendManifest = Join-Path $RepoRoot "github-alternative\backend\Cargo.toml"
    Write-Host "Checking / building spine-api binary..." -ForegroundColor Gray
    cargo build --manifest-path $BackendManifest --bin spine-api

    $ApiBin = Join-Path $RepoRoot "github-alternative\backend\target\debug\spine-api.exe"
    if (-not (Test-Path $ApiBin)) {
        throw "Binary not found at $ApiBin"
    }

    $Pids = @{}

    # Launch Backend
    $ApiArgs = @("--port", $Port, "--storage-root", $StorageRoot)
    if ($InMemory) {
        $ApiArgs += "--in-memory-db"
    }
    if ($CoordinatorAddr) {
        $ApiArgs += @("--coordinator-addr", $CoordinatorAddr)
    }

    Write-Host "Starting spine-api on 127.0.0.1:$Port (InMemory: $InMemory)..." -ForegroundColor Cyan
    $ApiLog = Join-Path $DevDir "spine-api.log"
    $ApiProc = Start-Process -FilePath $ApiBin -ArgumentList $ApiArgs -PassThru -RedirectStandardOutput $ApiLog -RedirectStandardError $ApiLog -WindowStyle Hidden
    $Pids["spine-api"] = $ApiProc.Id
    Start-Sleep -Seconds 1

    # Check health
    try {
        $health = Invoke-RestMethod -Uri "http://127.0.0.1:$Port/health/live" -TimeoutSec 5
        Write-Host "spine-api is healthy: $($health.status)" -ForegroundColor Green
    } catch {
        Write-Host "Warning: spine-api healthcheck failed. Check log: $ApiLog" -ForegroundColor Yellow
    }

    # Save PIDs
    $Pids | ConvertTo-Json | Set-Content -Path $PidFile

    Write-Host ""
    Write-Host "=== Spine Forge Development Environment Ready ===" -ForegroundColor Green
    Write-Host "  API Endpoint:       http://127.0.0.1:$Port"
    Write-Host "  Storage Directory:  $StorageRoot"
    Write-Host "  Health:             http://127.0.0.1:$Port/health/live"
    Write-Host "  API Docs:           docs/FORGE_API_REFERENCE.md"
    Write-Host "  Stop Command:       .\scripts\dev_forge.ps1 -Action stop"
}

switch ($Action) {
    "start" { Start-Forge }
    "stop" { Stop-Forge }
    "status" { Get-ForgeStatus }
    "clean" { Clean-Forge }
}
