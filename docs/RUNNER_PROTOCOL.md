# Spine Runner Protocol Specification (SPRP)

## 1. Protocol Overview

The **Spine Runner Protocol (SPRP)** defines the RESTful interaction contract between self-hosted/dedicated runner agents and the Spine Forge Control Plane.

All requests require authentication headers:
- `X-Spine-Runner-Id`: UUID of the registered runner.
- `X-Spine-Runner-Token`: Raw CSPRNG registration token (e.g. `spr_<hex>`).

---

## 2. API Endpoints

### 2.1 Register Runner
- **Endpoint**: `POST /api/v1/runners/register`
- **Auth**: Administrator session / PAT.
- **Request Body**:
```json
{
  "name": "windows-builder-01",
  "hostname": "build-vm-01.corp",
  "os": "windows",
  "arch": "x86_64",
  "labels": ["windows", "self-hosted", "msvc"],
  "max_concurrency": 2
}
```
- **Response** (`201 Created`):
```json
{
  "id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
  "name": "windows-builder-01",
  "token": "spr_f7819ad372e391bb3a201c...",
  "os": "windows",
  "arch": "x86_64",
  "labels": ["windows", "self-hosted", "msvc"]
}
```

### 2.2 Runner Heartbeat
- **Endpoint**: `POST /api/v1/runners/heartbeat`
- **Headers**: `X-Spine-Runner-Id`, `X-Spine-Runner-Token`
- **Request Body**:
```json
{
  "runner_id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
  "active_jobs": 1
}
```
- **Response**: `204 No Content`

### 2.3 Acquire Queued Job
- **Endpoint**: `POST /api/v1/runners/jobs/acquire`
- **Headers**: `X-Spine-Runner-Id`, `X-Spine-Runner-Token`
- **Request Body**:
```json
{
  "runner_id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
  "labels": ["windows", "self-hosted"]
}
```
- **Response** (`200 OK`):
```json
{
  "job": {
    "id": "job_99812",
    "run_id": "run_0012",
    "repo_id": "repo_44",
    "job_key": "build",
    "name": "Build Binary",
    "status": "RUNNING",
    "runner_id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
    "runs_on": ["windows", "self-hosted"],
    "needs": [],
    "attempt": 1,
    "fencing_token": 1,
    "lease_expires_at_nanos": 1730700000000000000
  },
  "steps": [
    {
      "id": "step_1",
      "job_id": "job_99812",
      "step_index": 0,
      "name": "Checkout Code",
      "status": "QUEUED"
    }
  ],
  "secrets": [
    {
      "id": "sec_1",
      "name": "SPINE_API_SECRET",
      "ciphertext": "super_token_99"
    }
  ]
}
```
*Note: Returns `null` if no matching jobs are currently queued.*

### 2.4 Ingest Log Chunk
- **Endpoint**: `POST /api/v1/runners/jobs/:id/logs`
- **Headers**: `X-Spine-Runner-Id`, `X-Spine-Runner-Token`
- **Request Body**:
```json
{
  "runner_id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
  "fencing_token": 1,
  "step_index": 0,
  "stream": "stdout",
  "content": "Compiling spine-core v0.1.0..."
}
```
- **Response**: `202 Accepted`

### 2.5 Complete Job
- **Endpoint**: `POST /api/v1/runners/jobs/:id/complete`
- **Headers**: `X-Spine-Runner-Id`, `X-Spine-Runner-Token`
- **Request Body**:
```json
{
  "runner_id": "e305e54c-1122-4876-b6fb-cfa2826cf6f0",
  "fencing_token": 1,
  "exit_code": 0,
  "step_results": [
    [0, 0]
  ]
}
```
- **Response**: `200 OK`
