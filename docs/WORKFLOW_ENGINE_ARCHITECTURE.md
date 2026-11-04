# Spine Forge Workflow Engine Architecture

## 1. Overview and Executive Summary

The **Spine Forge Workflow Engine** provides an integrated, cloud-native automation and continuous integration system directly within the Spine developer platform. It combines:
- Declarative workflow definitions stored in version control under `.spine/workflows/*.yml`.
- Native YAML & JSON syntax parsing with structural validation and line-precise diagnostics.
- A Directed Acyclic Graph (DAG) dependency resolution engine utilizing Tarjan's cycle detection algorithm.
- Fenced job lease coordination with monotonic attempt counters preventing split-brain execution across distributed runners.
- Seamless status synchronization with Forge Pull Request branch protection rules.

```
       +-------------------------------------------------------+
       |             Forge Control Plane (Spine API)           |
       |  +--------------------+     +-----------------------+ |
       |  | VCS Event Trigger  | --> | Workflow Parser & DAG | |
       |  | (Push / PR / Dispat|     | Tarjan Cycle Detection| |
       |  +--------------------+     +-----------------------+ |
       |                                         |             |
       |                                         v             |
       |  +--------------------+     +-----------------------+ |
       |  | Webhook Dispatcher | <-- | Scheduler & State     | |
       |  | (Transactional OB) |     | (Queued/Running/Succ) | |
       |  +--------------------+     +-----------------------+ |
       +-----------------------------------------+-------------+
                         ^                       |
         Job Lease Poll  |                       | Fenced Job Assignment
         & Heartbeat     |                       v & Secret Injection
       +-----------------+-----------------------+-------------+
       |                 Dedicated Isolated Runner             |
       |  +-------------------------------------------------+  |
       |  | - Process-Tree Sandbox / Clean Env              |  |
       |  | - Hard Step Timeout Enforcement                 |  |
       |  | - Realtime Console Streaming with Secret Mask   |  |
       |  +-------------------------------------------------+  |
       +-------------------------------------------------------+
```

---

## 2. Core Architectural Subsystems

### 2.1 Workflow Definition & Ingestion Plane
- Workflow files reside in repository trees at `.spine/workflows/*.yml` or `.spine/workflows/*.yaml`.
- Upon commit push or manual synchronization, the Forge API invokes `WorkflowService::sync_workflows_from_tree`.
- Each definition is validated for syntactic soundness and structural DAG validity (zero missing dependencies, zero cycles).
- Content is cryptographically hashed with SHA-256 to ensure immutable auditability across subsequent runs.

### 2.2 Event Triggering & Filter Evaluation
- Workflows declare trigger hooks in their `on:` block:
  - `push.branches`: Branch filters with glob matching (`main`, `release/*`, `*`).
  - `pull_request.branches`: Pull request targeting matching base branches.
  - `workflow_dispatch`: Manual operator or API-initiated invocations.
- When an authorized VCS event occurs, matching definitions instantiate an immutable `WorkflowRun` in `QUEUED` status.
- Downstream Pull Request Check results are initialized in `PENDING` status.

### 2.3 DAG Engine & Dependency Scheduling
- Each job declared in a workflow specifies optional prerequisites via `needs: [job_a, job_b]`.
- Initial State: Jobs with empty `needs` are marked `QUEUED`. Jobs with unsatisfied prerequisites are marked `WAITING_FOR_RUNNER`.
- As runner workers complete upstream jobs with exit code 0 (`SUCCEEDED`), the `WorkflowScheduler::tick` inspects dependent jobs. Once all prerequisites report `SUCCEEDED`, the dependent job automatically transitions to `QUEUED`.
- If any prerequisite fails (`FAILED`, `TIMED_OUT`, or `CANCELLED`), downstream dependent jobs transition to `SKIPPED`.

### 2.4 Fenced Leases & Anti-Split-Brain Concurrency
- Runners poll for work via `acquire_job(runner_id, labels)`.
- When a job is assigned, the engine atomically:
  1. Increments `fencing_token = fencing_token + 1`.
  2. Sets `lease_expires_at_nanos = now + 5_minutes`.
  3. Transitions job status to `RUNNING` assigned to `runner_id`.
- If a runner partitions or experiences an unrecoverable crash, its lease expires. The scheduler marks the job `TIMED_OUT` or re-queues it.
- Stale completion reports or log chunks bearing outdated fencing tokens are strictly rejected with `HTTP 401 / ForgeError::Unauthorized`.

---

## 3. Pull Request Check Enforcement Boundary

The workflow engine functions as an authoritative gatekeeper for code review and repository merges:
1. When a workflow executes for a commit referenced by an open Pull Request, its name and run ID are recorded in `workflow_check_results`.
2. Branch protection rules declare `required_status_checks: Vec<String>`.
3. In `PermissionService::check_merge_allowed`, before any merge transaction can be authorized, the target commit is evaluated against all required checks.
4. If any required check is absent, `PENDING`, or `FAILURE`, the merge is blocked with `422 Unprocessable Entity`.
