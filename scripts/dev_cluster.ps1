<#
.SYNOPSIS
    Starts or stops a local 4-process Spine development cluster.
    - 1 Coordinator (127.0.0.1:9000)
    - 3 Storage Nodes (127.0.0.1:9001, 9002, 9003)
.EXAMPLE
    .\scripts\dev_cluster.ps1 -Action start
    .\scripts\dev_cluster.ps1 -Action stop
    .\scripts\dev_cluster.ps1 -Action status
#>
param (
    [ValidateSet("start", "stop", "status", "clean")]
    [string]$Action = "start"
)

$ErrorActionPreference = "Stop"
$ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot = Split-Path -Parent $ScriptDir
$ClusterDir = Join-Path $RepoRoot ".spine-dev-cluster"
$PidFile = Join-Path $ClusterDir "pids.json"

function Clean-Cluster {
    Write-Host "Stopping any running cluster processes..." -ForegroundColor Yellow
    Stop-Cluster
    if (Test-Path $ClusterDir) {
        Write-Host "Removing $ClusterDir..." -ForegroundColor Yellow
        Remove-Item -Recurse -Force $ClusterDir
    }
    Write-Host "Cluster cleaned." -ForegroundColor Green
}

function Stop-Cluster {
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
    # Fallback cleanup by name
    Get-Process spine-coordinator, spine-storage-node -ErrorAction SilentlyContinue | ForEach-Object {
        Write-Host "Stopping orphan $($_.ProcessName) (PID $($_.Id))..."
        Stop-Process -Id $_.Id -Force
    }
    Write-Host "Cluster stopped." -ForegroundColor Green
}

function Get-ClusterStatus {
    if (-not (Test-Path $PidFile)) {
        Write-Host "Cluster is NOT running (no PID file at $PidFile)." -ForegroundColor Red
        return
    }
    $pids = Get-Content $PidFile | ConvertFrom-Json
    Write-Host "=== Spine Development Cluster Status ===" -ForegroundColor Cyan
    foreach ($prop in $pids.PSObject.Properties) {
        $procId = $prop.Value
        $procName = $prop.Name
        $running = Get-Process -Id $procId -ErrorAction SilentlyContinue
        if ($running) {
            Write-Host "  $procName (PID $procId): RUNNING" -ForegroundColor Green
        } else {
            Write-Host "  $procName (PID $procId): DEAD" -ForegroundColor Red
        }
    }
}

function Start-Cluster {
    Write-Host "Building spine-coordinator and spine-storage-node binaries..." -ForegroundColor Cyan
    cargo build --manifest-path "$RepoRoot\git-alternative\Cargo.toml" --bin spine-coordinator --bin spine-storage-node
    if ($LASTEXITCODE -ne 0) {
        Write-Error "Cargo build failed."
        return
    }

    $CoordinatorBin = Join-Path $RepoRoot "git-alternative\target\debug\spine-coordinator.exe"
    $StorageNodeBin = Join-Path $RepoRoot "git-alternative\target\debug\spine-storage-node.exe"

    if (-not (Test-Path $ClusterDir)) {
        New-Item -ItemType Directory -Path $ClusterDir | Out-Null
    }

    $CoordStore = Join-Path $ClusterDir "coordinator_state.json"
    $CoordLog = Join-Path $ClusterDir "coordinator.log"

    Write-Host "Generating enrollment tokens for 3 storage nodes..." -ForegroundColor Cyan
    $Tokens = @()
    for ($i = 1; $i -le 3; $i++) {
        $tokenOutput = & $CoordinatorBin --store-path $CoordStore generate-token --node-id "node-0$i" --validity-hours 24
        $token = ($tokenOutput | Select-String "Enrollment Token: ([a-f0-9]+)").Matches.Groups[1].Value
        if (-not $token) {
            # Direct parse fallback
            $token = ($tokenOutput -split ":\s*")[-1].Trim()
        }
        $Tokens += $token
        Write-Host "  Node-0$i token: $token"
    }

    Write-Host "Starting Coordinator on 127.0.0.1:9000..." -ForegroundColor Cyan
    $coordProc = Start-Process -FilePath $CoordinatorBin `
        -ArgumentList "--listen 127.0.0.1:9000 --store-path `"$CoordStore`" --heartbeat-timeout-secs 10 --suspect-timeout-secs 5" `
        -RedirectStandardOutput $CoordLog `
        -RedirectStandardError (Join-Path $ClusterDir "coordinator_err.log") `
        -PassThru

    Start-Sleep -Milliseconds 800

    $NodePids = @{}
    $NodePids["coordinator"] = $coordProc.Id

    for ($i = 1; $i -le 3; $i++) {
        $nodeId = "node-0$i"
        $port = 9000 + $i
        $nodeRoot = Join-Path $ClusterDir "node-$i"
        $zone = "zone-$([char](96 + $i))"
        $rack = "rack-01"
        $token = $Tokens[$i - 1]
        $nodeLog = Join-Path $ClusterDir "node-$i.log"

        if (-not (Test-Path $nodeRoot)) {
            New-Item -ItemType Directory -Path $nodeRoot | Out-Null
        }

        Write-Host "Starting Storage Node $nodeId on 127.0.0.1:$port (zone=$zone, rack=$rack)..." -ForegroundColor Cyan
        $nodeProc = Start-Process -FilePath $StorageNodeBin `
            -ArgumentList "--node-id $nodeId --storage-root `"$nodeRoot`" --listen 127.0.0.1:$port --coordinator-addr 127.0.0.1:9000 --zone $zone --rack $rack --enrollment-token $token --heartbeat-interval-secs 2" `
            -RedirectStandardOutput $nodeLog `
            -RedirectStandardError (Join-Path $ClusterDir "node-$i`_err.log") `
            -PassThru

        $NodePids[$nodeId] = $nodeProc.Id
    }

    $NodePids | ConvertTo-Json | Set-Content -Path $PidFile
    Write-Host "Spine cluster successfully launched!" -ForegroundColor Green
    Write-Host "  Coordinator: 127.0.0.1:9000"
    Write-Host "  Storage Nodes: 127.0.0.1:9001, 127.0.0.1:9002, 127.0.0.1:9003"
    Write-Host "  Logs & State stored in: $ClusterDir"
}

switch ($Action) {
    "start"   { Start-Cluster }
    "stop"    { Stop-Cluster }
    "status"  { Get-ClusterStatus }
    "clean"   { Clean-Cluster }
}
