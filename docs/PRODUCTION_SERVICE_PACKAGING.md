# Spine Production Service Packaging & Process Hardening

## 1. Service Profiles

Spine deployment supports two operational profiles:

### Profile A: Core Forge (Production Default)
- **Role:** Web UI, Git Smart HTTP, Git-over-SSH, Git LFS, Package Registry, Repository Management.
- **Runner Policy:** `UNTRUSTED_WORKFLOW_RUNNER_ENABLED=false`.
- **Security Requirement:** Untrusted CI workflow execution is strictly prohibited from running on the control plane or host OS. All workflow jobs submitted by untrusted users remain queued until an isolated worker node claims them.

### Profile B: Isolated CI Worker (Dedicated Compute)
- **Role:** Executes untrusted user scripts, builds, and test containers.
- **Isolation Requirement:** Uses microVM (Firecracker, Cloud Hypervisor) or strictly partitioned OCI sandbox containers with rootless namespaces, seccomp-bpf filters, and memory/CPU limits.

---

## 2. Container Image Hardening

The production `Dockerfile.production` implements defense-in-depth principles:
1. **Multi-Stage Build:**
   - Build environment (Rust toolchain, compilers, build scripts) discarded after compilation.
   - Final runtime base image is `debian:bullseye-slim` containing only minimal C runtime libraries and CA certificates.
2. **Unprivileged Execution:**
   - Runs under dedicated service account `spine:spine` (UID 10001, GID 10001).
   - Never runs as `root`.
3. **Container Health Checks:**
   - Built-in `HEALTHCHECK` targeting `/healthz` on port 3000 every 10 seconds.
4. **Zero Embedded State:**
   - Database passwords, private keys, and repository files are mounted from persistent volumes or secret injection mechanisms.

---

## 3. Systemd Process Isolation

When deployed on bare-metal Linux or standard VMs without containers, the systemd unit `systemd/spine-api.service` enforces Linux kernel-level sandboxing:
- `NoNewPrivileges=true`: Disallows child processes from gaining elevated permissions via `setuid`/`setgid`.
- `ProtectSystem=strict`: Mounts `/usr`, `/boot`, `/etc`, and system directories read-only.
- `ProtectHome=true`: Prevents access to `/home`, `/root`, and `/run/user`.
- `PrivateTmp=true`: Isolates `/tmp` in an ephemeral per-process namespace.
- `PrivateDevices=true`: Restricts access to raw hardware devices (`/dev`).
- `ProtectKernelTunables=true` & `ProtectKernelModules=true`: Prevents modifications to kernel parameters or module loading.
- `ReadWritePaths=/var/lib/spine`: Restricts all filesystem write operations exclusively to the Spine application directory.
