# RUNNER SECURITY LIMITATIONS & PLATFORM REALITY

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. WINDOWS PLATFORM REALITY

1. **Lack of Native Linux-Style Namespaces:** Windows does not provide unprivileged mount or network namespaces natively without virtualization (Hyper-V/WSL2).
2. **Job Objects vs Sandboxing:** Windows Job Objects provide memory limits and process-tree grouping, but do not sandbox filesystem access or network connections.
3. **Fail-Closed Enforcement:** Because bare Windows processes cannot be safely sandboxed against arbitrary filesystem inspection, Spine strictly refuses to execute untrusted code in `RunnerSandboxMode::Unsandboxed` mode on Windows.
4. **Trusted Mode Requirement:** Developers running local workflows on Windows workstations must explicitly acknowledge `RunnerSandboxMode::TrustedLocalDevelopment`.

---

## 2. PRODUCTION DEPLOYMENT RECOMMENDATION

For multi-tenant public repository hosting:
1. Runners must run on dedicated, disposable Linux VMs with Ephemeral Root disks.
2. Workflows must execute inside rootless OCI containers (Docker / Podman / gVisor) with network egress firewalled to prevent metadata and internal IP access.
3. No runner VM may share networks with the Spine PostgreSQL database or coordinator internal admin ports.
