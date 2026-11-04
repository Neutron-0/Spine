# RUNNER ISOLATION & HOST SECURITY ARCHITECTURE

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. SUMMARY OF CONTROLS IMPLEMENTED

1. **Explicit Sandbox Boundary:** `IsolatedRunnerExecutor` now validates `sandbox_mode` before running untrusted workflows.
2. **Fail-Closed Policy:** Untrusted workflows fail closed unless running within a verified container isolation backend (`RunnerSandboxMode::SandboxedOci`).
3. **Fencing Token Rejection:** Stale runner heartbeats, log uploads, and job completions are rejected using strict monotonic compare-and-swap fencing tokens.
4. **Environment Cleansing:** Only essential system variables (`PATH`, `SYSTEMROOT`, `COMSPEC`, `PATHEXT`, `TEMP`, `TMP`, `USERPROFILE`, `HOME`) are forwarded; sensitive host variables are stripped.
5. **Process-Tree Termination:** Timed-out jobs are forcefully terminated along with all child processes using `taskkill /PID <pid> /T /F` on Windows and `SIGKILL` to process groups on POSIX.
6. **Masking Engine:** Pre-ingestion redaction of all known workflow secrets across stdout/stderr streams.
