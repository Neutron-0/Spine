# Spine Build Runner Architecture

## 1. Architectural Philosophy & Isolation Boundary

A core security tenet of Spine Forge is the **strict separation between the Control Plane and the Runner Execution Plane**.

The Spine Forge API server, coordinator, storage nodes, and database MUST NEVER execute arbitrary user scripts or compiled build artifacts directly on control plane hardware.

Instead, build workloads are dispatched exclusively to dedicated, isolated runner agents:
- Runners operate in isolated workspaces.
- Runners poll the Forge API over authenticated HTTPS/REST channels.
- Environment variables are strictly sanitized.
- Child process trees are bounded by aggressive timeouts.
- Output streams are masked against repository and organization secrets.

---

## 2. Runner Lifecycle & Protocol State Machine

```
   [Offline]
       |
       | 1. POST /api/v1/runners/register (returns CSPRNG token)
       v
   [ACTIVE] <-----------------------------+
       |                                  |
       | 2. POST /api/v1/runners/heartbeat|
       |                                  |
       | 3. POST /api/v1/runners/jobs/acquire
       v                                  |
   [ASSIGNED / RUNNING]                   |
       |                                  |
       | 4. Workspace Sandbox & Exec      |
       | 5. POST .../jobs/:id/logs        |
       | 6. POST .../jobs/:id/complete ---+
       v
   [IDLE]
```

---

## 3. Runner Agent Components

### 3.1 Registration & Identity
1. An administrator creates a runner via `POST /api/v1/runners/register` providing:
   - Runner Name: Human-readable identifier.
   - Hostname, OS, and Architecture (`windows/x86_64`, `linux/x86_64`).
   - Labels: e.g. `[windows, self-hosted, gpu]`.
   - Max Concurrency: Maximum concurrent jobs supported on the runner host.
2. The server generates a 32-byte CSPRNG token prefixed with `spr_`.
3. The server computes and stores only the SHA-256 hash of the token (`token_hash`), returning the raw token to the runner once.

### 3.2 Polling & Work Acquisition
- The runner agent polls `POST /api/v1/runners/jobs/acquire` sending its labels.
- The server checks `list_queued_workflow_jobs` where all `runs_on` requirements match the runner's declared labels.
- When matching work is found, the server grants a 5-minute initial lease, increments the job's `fencing_token`, and returns the job definition, steps, and masked secrets.

### 3.3 Sandboxed Execution
- The runner allocates an isolated workspace: `runner_workspaces/<job_id>`.
- The runner clears the parent process environment, preserving only essential OS variables (`PATH`, `SYSTEMROOT`, `COMSPEC`, `TEMP`).
- Steps are executed in sequence. If any step fails (exit code != 0 or timeout), the remaining steps are aborted and marked `SKIPPED`.
- Upon termination, the workspace is cleaned up, and a completion report is transmitted to the Forge API.
