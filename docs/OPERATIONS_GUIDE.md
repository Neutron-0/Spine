# SPINE OPERATIONS & SYSTEM ADMINISTRATION GUIDE

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. COMPONENT ARCHITECTURE OVERVIEW

Spine comprises the following standalone, cooperating binaries and services:

1. **`spine` (CLI):** Developer workstation command-line tool.
2. **`spine-daemon`:** SPWP binary wire protocol repository server (default port: `9418` / `9419` TLS).
3. **`spine-coordinator`:** Cluster coordinator managing repository replica placement and epoch fencing.
4. **`spine-storage-node`:** Storage daemon hosting native `.spine/` packfiles, chunks, and replica catalogs.
5. **`spine-api`:** Forge HTTP REST API, webhook worker, workflow scheduler, and search engine.
6. **`spine-forge-frontend`:** React 18 / TypeScript single-page application.

---

## 2. STANDARD OPERATING PROCEDURES

### 2.1 Starting the Forge Backend API
```powershell
# Set configuration environment variables
$env:PORT = "8080"
$env:DATABASE_URL = "postgres://spine_user:secret@localhost:5432/spine_forge"
$env:STORAGE_ROOT = "D:\Vault\Spine\storage"

# Run spine-api binary
cargo run --manifest-path github-alternative/backend/Cargo.toml -p spine-api
```

### 2.2 Registering and Running a Workflow Worker
```powershell
# Register runner
$REG_JSON = Invoke-RestMethod -Method Post -Uri "http://localhost:8080/api/v1/runners/register" -ContentType "application/json" -Body '{"name":"worker-01","hostname":"runner-host","os":"windows","arch":"x86_64","labels":["windows","rust"],"max_concurrency":2}'
$RUNNER_TOKEN = $REG_JSON.token
$RUNNER_ID = $REG_JSON.runner.id

# Runner polls for jobs via acquire endpoint
Invoke-RestMethod -Method Post -Uri "http://localhost:8080/api/v1/runners/jobs/acquire" -Headers @{"Authorization"="Bearer $RUNNER_TOKEN"} -ContentType "application/json" -Body '{"runner_id":"'$RUNNER_ID'","labels":["windows","rust"]}'
```

### 2.3 Running Repository Garbage Collection
```powershell
# Local repository GC
spine gc --prune-now

# Repack loose objects into packfile
spine repack --max-pack-size 100M
```
