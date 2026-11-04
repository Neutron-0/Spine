# Runner Isolation & Hostile Execution Defense Model

## 1. Threat Model & Adversarial Posture

Spine CI/CD workflows run untrusted code authored by external contributors, dependencies downloaded from untrusted registries, and potentially adversarial automated payloads.

The runner defense model addresses the following threat vectors:
1. **Control Plane Compromise**: Untrusted repository scripts escaping execution sandboxes to compromise the Forge API or database.
2. **Secret Exfiltration**: Scripts attempting to echo, dump, or transmit stored repository secrets or credentials to third-party endpoints.
3. **Infinite Loops & Resource Exhaustion (DoS)**: Workflows running unbounded computations or spawning fork bombs.
4. **Split-Brain Replay Attacks**: Partitioned runners waking up late and attempting to overwrite state.
5. **Cross-Job State Pollution**: Subsequent jobs reading left-over files or tokens from previous executions.

---

## 2. Security Controls & Defenses

### 2.1 Workspace Isolation & Ephemeral Lifecycles
- Every acquired job creates a uniquely indexed workspace: `runner_workspaces/<job_id>`.
- The child process's working directory is bound strictly to this path.
- Upon job conclusion, the workspace directory and all contained temporary files are recursively purged from disk.

### 2.2 Strict Environment Sanitization
- The parent runner environment is **completely cleared** via `Command::env_clear()`.
- Only a whitelist of strictly necessary OS paths are preserved:
  - Windows: `PATH`, `SYSTEMROOT`, `COMSPEC`, `TEMP`, `TMP`, `USERPROFILE`, `NUMBER_OF_PROCESSORS`, `OS`.
  - Unix: `PATH`, `HOME`, `TEMP`, `USER`.
- Control plane configuration, database connection strings, coordinator credentials, and Forge master keys are never accessible within child process environments.

### 2.3 Hard Step Timeouts & Process-Tree Termination
- Every step has an enforced hard execution ceiling (default: 60 seconds for step-level operations, configurable up to job timeout).
- If the timeout expires:
  - On Windows: The runner invokes `taskkill /PID <pid> /T /F` to ensure all descendant processes and background sub-processes in the process tree are forcibly killed.
  - On Unix: The process group is signaled with `SIGKILL`.
  - The step is marked `timed_out: true` with exit code `124`.

### 2.4 Secret Masking
- Secrets stored in `workflow_secrets` are injected as environment variables.
- As logs are captured from stdout and stderr, the runner and server cross-reference all registered secret ciphertexts.
- Any occurrence of a secret value (3 characters or longer) is dynamically replaced with `***` before persisting or streaming to the UI.

### 2.5 Fencing Tokens & Lease Invalidation
- When a runner acquires a job, it receives a monotonically increasing `fencing_token`.
- If a runner hangs and its lease expires, the scheduler reassigns the job, incrementing the fencing token.
- Stale completion reports or log chunks bearing outdated fencing tokens are strictly rejected with `HTTP 401 / ForgeError::Unauthorized`.
