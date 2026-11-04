# ADR-0011: Spine Forge Automation Platform, Workflow Engine, and Runner Security Model

## Status
Accepted

## Context
Spine Forge requires a native developer automation platform capable of executing continuous integration, build tests, automated packaging, and release pipelines directly in response to repository events (pushes, pull requests, and manual dispatches).

A fundamental challenge of version control CI/CD systems is that repository-provided code and workflow scripts are inherently **untrusted**. Executing arbitrary user code within the core control plane (the Forge API, the distributed coordinator, storage nodes, or database server) presents catastrophic security risks, including credential exfiltration, database corruption, denial of service, and lateral privilege escalation across tenants.

Furthermore, remote webhooks connecting Forge to external developer systems must deliver events reliably without introducing Server-Side Request Forgery (SSRF) vulnerabilities into private infrastructure.

---

## Decisions

### 1. Hard Boundary: Control Plane vs. Runner Execution Plane
- **Forge Control Plane (`spine-api`)**: Responsible exclusively for authentication, authorization, workflow file validation, scheduling DAGs, tracking job state transitions, collecting streaming logs, and issuing scoped capabilities. The control plane **never** executes repository shell commands or user scripts.
- **Runner Execution Plane (`spine-runner`)**: An isolated worker process that runs outside the API server. Runners authenticate with dedicated runner credentials, poll or receive job assignments, acquire short-lived job-scoped capability tickets, execute builds within isolated workspaces, and report progress back to the control plane over an authenticated protocol.

### 2. Workflow Definition Format & Immutability
- Workflows are defined in YAML under `.spine/workflows/<workflow_name>.yml`.
- Schema supports: workflow name, triggers (`push`, `pull_request`, `workflow_dispatch`), branch/path filters, job dependency DAG (`needs: [...]`), runner requirements (`runs-on: [...]`), environment variables, timeouts, permissions, and sequential shell steps.
- **Run Immutability**: When a workflow is triggered, the control plane parses the workflow file at the triggering commit, computes a BLAKE3 content digest, and stores the snapshot in `workflow_runs`. Future edits to the repository's workflow file cannot mutate active or past runs.

### 3. Job Dependency DAG & State Machine
- Jobs within a workflow declare dependencies using `needs`. Cycles are detected at parse time via depth-first cycle detection and rejected.
- Valid job states:
  $$\text{QUEUED} \to \text{WAITING\_FOR\_RUNNER} \to \text{ASSIGNED} \to \text{STARTING} \to \text{RUNNING} \to \{\text{SUCCEEDED}, \text{FAILED}, \text{TIMED\_OUT}, \text{CANCELLED}\}$$
- Failure in a prerequisite job automatically marks downstream dependent jobs as skipped, unless explicit `if: always()` conditions are specified.

### 4. Runner Registration, Leases & Fencing Tokens
- Runners enroll via dedicated runner registration tokens and receive unique runner IDs and private credentials.
- When a job is assigned to a runner, the control plane generates a **fencing token** (monotonically increasing attempt number) and a time-bounded **lease**.
- Stale or partition-delayed runners attempting to report logs or status with an expired lease or superseded fencing token are strictly rejected, preventing stale runners from corrupting newer retry attempts.
- Runner heartbeats are tracked every 10 seconds. If a runner fails to heartbeat within 30 seconds, its active jobs transition to `LOST` / `RECOVERY_REQUIRED` and become eligible for rescheduling.

### 5. Runner Workspace Isolation & Security Controls
- **Disposable Workspaces**: Every job executes in a temporary, isolated working directory (`runner_workspaces/<job_id>`) that is purged immediately after execution.
- **Process Tree Management**: On Windows, runners create dedicated job process trees and enforce hard timeouts. When a job times out or is cancelled, the entire child process hierarchy is terminated.
- **Credential Sanitization**: Build environments never inherit Forge API tokens, database connection strings, or coordinator credentials. Only explicitly declared secrets are injected.
- **Fork Pull Request Restrictions**: Workflows triggered by pull requests from untrusted forks are denied access to write-scoped secrets and privileged deployment environments without explicit maintainer approval.

### 6. Durable Webhook Architecture & SSRF Defenses
- Webhook deliveries use an append-only transactional outbox pattern (`webhook_outbox`). Events are emitted only after authoritative operations commit to the database.
- A dedicated background worker claims pending events, signs payloads using HMAC-SHA256 (`X-Spine-Signature-256`), and delivers over HTTP/HTTPS with bounded timeouts and exponential backoff retry.
- **SSRF Defense**: User-configured webhook URLs are inspected before connection. Destination IP addresses resolving to loopback (`127.0.0.0/8`, `::1`), private ranges (`10.0.0.0/8`, `172.16.0.0/12`, `192.168.0.0/16`), link-local (`169.254.0.0/16`), AWS/GCP metadata (`169.254.169.254`), or internal cluster ports are rejected at validation and connection time.

### 7. Pull Request Check Enforcement
- Check runs are cryptographically bound to the exact target commit SHA/OID.
- When branch protection requires status checks, the authoritative write pipeline (`AuthoritativeRepositoryWriter`) queries the database and verifies that all required checks have succeeded on the target commit before executing the merge.

---

## Consequences
- Clean separation of concerns between control plane and runner execution plane.
- Hostile workflow code cannot compromise Forge infrastructure.
- Webhook delivery is durable, auditable, and resilient to transient network failures.
- Native Windows 11 development is supported out of the box with job process isolation and workspace cleanup.
