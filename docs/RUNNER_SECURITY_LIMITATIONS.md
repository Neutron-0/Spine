# Runner Security Boundaries, Sandbox Architecture, and Limitations

Date: 2026-11-04  
System: SPINE Forge Runner Isolation Framework  

---

## 1. Architectural Scope and Design Boundaries

Spine Forge runners execute workflow steps driven by user-authored YAML definitions. Because workflows can originate from public repository forks, open pull requests, or external contributors, the runner execution environment enforces multi-tiered defensive barriers.

However, bare-metal or process-level isolation on general-purpose host OS environments (Windows / Linux) inherently carries specific boundaries when compared to hardware-virtualized microVMs. This document defines the exact capabilities, protections, and current architectural limitations of the Spine runner engine.

---

## 2. Implemented Defense In Depth

### 2.1 Process Isolation & Tree Destruction
- **Child Process Sandboxing**: Steps are executed in dedicated subprocesses (`cmd.exe` on Windows, `/bin/sh` on Unix) spawned directly into an isolated workspace directory (`runner_workspaces/<job_id>`).
- **Complete Environment Clearing**: The host environment is wiped (`env_clear()`). Sensitive host credentials, database connection strings, TLS keys, and cluster administrative tokens are strictly inaccessible.
- **Process Tree Killing**: Timeouts trigger recursive process tree termination (`taskkill /PID <pid> /T /F` on Windows; process group termination on Unix) preventing orphaned daemon escapes.

### 2.2 Strict Ephemeral Workspaces
- Workspaces are ephemeral: created per job and recursively removed immediately upon job completion or abort.
- Workspace paths outside the allocated runner workspace directory are prohibited.

### 2.3 Untrusted Pull Request Secret Gating
- Workflows triggered by `pull_request` events originating from fork repositories or external non-collaborators are denied access to repository secrets by policy.
- Masking filters stdout and stderr in-flight to replace any accidental secret emissions with `***`.

### 2.4 Fencing Token Defense
- Distributed runners use monotonic fencing tokens. If a runner experiences a network partition or hangs past its lease duration, any delayed state updates are rejected with `401 Unauthorized`.

---

## 3. Operational Limitations & Production Recommendations

### 3.1 Kernel & OS Boundary Limitations
- **Kernel Sharing**: Standard subprocess execution shares the underlying OS kernel. While memory and environment variables are segregated, kernel exploits could theoretically impact the host.
- **Recommended Production Deployment**: For multi-tenant or untrusted public hosting:
  - Deploy runners inside ephemeral microVMs (e.g., AWS Firecracker, Cloud Hypervisor, or disposable QEMU VMs) or isolated container runtimes (gVisor / Kata Containers).
  - Use dedicated, non-privileged service accounts with no administrator/root privileges.

### 3.2 Network Egress Filtering
- The runner currently inherits host network access unless constrained by host firewall rules (Windows Filtering Platform / iptables).
- For strict defense, outbound traffic should be routed through an egress proxy enforcing a default-deny policy with whitelisted endpoints (e.g., official package registries like crates.io, npmjs.org).

### 3.3 Fail-Closed Principle
- If workspace creation fails, directory permissions are invalid, or process spawning fails, the step immediately fails closed without attempting fallbacks or elevated retries.
