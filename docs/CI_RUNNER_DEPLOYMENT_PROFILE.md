# Isolated CI Runner Deployment Profile Specification

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Profile Identifier:** `spine-ci-runner-v1`  
**Security Boundary:** Low-Trust Isolated Execution Sandbox (Zero Access to Core Forge Secrets or DB)

---

## 1. Threat Model & Isolation Architecture

The CI Runner subsystem executes untrusted developer code, automated test suites, pull request validations, and build steps defined in `.spine/workflows/*.yaml`.

### Security Invariants:
1. **Network Demilitarization:** CI runners reside in a segregated worker VPC/subnet. Runners cannot access the PostgreSQL database port (`5432`), internal storage node ports (`9101`), or cluster coordinator ports (`8400`).
2. **Pull-Only Job Polling:** Runners initiate outbound TLS connections to the Core Forge public HTTPS API (`POST /api/v1/runners/jobs/claim`). Core Forge never connects inbound to runners.
3. **Mutual Token Authentication:** Each runner authenticates using an organization- or repository-scoped runner token (`spine_rt_...`) hashed with Argon2id.
4. **Zero Shared Storage:** Runners maintain ephemeral scratch directories. Workspace directories are cleaned after each job run.
5. **Hypervisor / MicroVM Isolation:** For enterprise production, runners execute jobs inside isolated Docker containers, Podman rootless sandboxes, or Firecracker microVMs.

```text
+------------------------------------+             +------------------------------------+
|         Core Forge Subnet          |             |       Isolated CI Runner Subnet    |
|                                    |             |                                    |
|  +------------------------------+  |             |  +------------------------------+  |
|  |          spine-api           |  |             |  |         spine-runner         |  |
|  |  (Job Queue, Token Validator)|  | <========== |  | (Polls jobs, runs sandboxes) |  |
|  +------------------------------+  | HTTPS: 443  |  +------------------------------+  |
|                 |                  | Outbound    |                 |                  |
|                 v                  | Only        |                 v                  |
|  +------------------------------+  |             |  +------------------------------+  |
|  | PostgreSQL & Bare Repo Disks |  |             |  | Ephemeral Docker / Firecracker| |
|  | (STRICTLY BLOCKED TO RUNNERS)|  |             |  | Container Sandbox            |  |
|  +------------------------------+  |             |  +------------------------------+  |
+------------------------------------+             +------------------------------------+
```

---

## 2. Runner Configuration (`/etc/spine/runner.env`)

```bash
# Runner Instance Identity
SPINE_RUNNER_NAME=worker-node-us-east-01
SPINE_RUNNER_LABELS=linux,x86_64,docker,rust

# Core Forge API Endpoint
SPINE_FORGE_URL=https://forge.example.com
SPINE_RUNNER_REGISTRATION_TOKEN=spine_rt_9876543210fedcba9876543210

# Concurrency & Resource Limits
SPINE_RUNNER_MAX_CONCURRENT_JOBS=4
SPINE_RUNNER_JOB_TIMEOUT_SECONDS=1800
SPINE_RUNNER_SCRATCH_DIR=/var/run/spine-runner/workspace

# Execution Backend
SPINE_RUNNER_BACKEND=docker
SPINE_RUNNER_DOCKER_IMAGE=rust:1.80-slim
SPINE_RUNNER_DOCKER_NETWORK=none
SPINE_RUNNER_MEMORY_LIMIT=4G
SPINE_RUNNER_CPU_LIMIT=2.0
```

---

## 3. Sandboxing & Escape Prevention

When executing job steps, the runner applies OS cgroup and container constraints:
- **`--cap-drop=ALL`**: Drops all Linux capabilities inside job containers.
- **`--security-opt=no-new-privileges`**: Prevents setuid binaries from escalating permissions.
- **Read-Only Root Filesystem**: Mounts container root read-only; writes allowed only to tmpfs `/tmp` and workspace `/build`.
- **Network Isolation**: By default, build steps without declared network requirements execute with `--network=none` to prevent secret exfiltration to third-party endpoints.
