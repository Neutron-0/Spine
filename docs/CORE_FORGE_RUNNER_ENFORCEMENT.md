# Core Forge CI/CD Runner Policy & Security Enforcement

**Module:** `spine_api::runner`, `spine_api::workflow`  
**Deployment Profile:** `core-forge-v1`  
**Verification Suite:** `tests/profile_enforcement_test.rs`  
**Milestone:** Batch 21 Remediation (Finding F-03)

---

## 1. Problem Statement & Threat Model

While Spine Forge architecture documents indicated that Core Forge (`core-forge-v1`) does not support untrusted external code execution, the application code had no programmatic gate. An untrusted pull request from a fork could trigger a workflow run, which an unsandboxed local runner would execute directly on the host machine.

---

## 2. Programmatic Enforcement Architecture

The `DeploymentProfile::CoreForge` profile is injected at startup across all workflow and runner components:

### 2.1 Workflow Trigger Filtering (`WorkflowService`)
In `WorkflowService::trigger_workflows`:
- Events matching `pull_request_fork`, `fork`, or `untrusted_*` are rejected under `core-forge-v1`.
- No `WorkflowRun` or `WorkflowJob` records are created in PostgreSQL or queued in the scheduler.

### 2.2 Runner Registration Security (`RunnerService`)
In `RunnerService::register_runner`:
- Any runner registration that includes labels `untrusted`, `shared`, or `public` is rejected with `ForgeError::Forbidden`:
  > *"Untrusted or shared runner registration is disallowed under deployment profile 'core-forge-v1'"*

### 2.3 Job Dispatch Defense (`RunnerService::acquire_job`)
- Fails to match or acquire jobs flagged as untrusted.

### 2.4 Runner Host Execution Fail-Closed (`IsolatedRunnerExecutor`)
In `IsolatedRunnerExecutor::execute_job_with_trust`:
- If `is_untrusted == true` and `profile.allow_untrusted_ci() == false`, the executor immediately aborts without spawning a child process:
  ```rust
  return (
      1,
      vec![StepExecutionResult {
          step_index: 0,
          exit_code: 1,
          stdout: String::new(),
          stderr: "SECURITY ERROR: Untrusted workflow execution is strictly disabled on this deployment profile ('core-forge-v1').".to_string(),
          timed_out: false,
      }],
  );
  ```

### 2.5 User Interface Transparency (`github-alternative/frontend`)
- Actions and Runners views display security badges:
  - *"Core Forge Security: Untrusted / Fork CI execution is strictly disabled."*
  - *"Only authenticated organization runners are permitted."*
