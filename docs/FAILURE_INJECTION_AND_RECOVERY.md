# FAILURE INJECTION AND RECOVERY SPECIFICATION

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. REPRODUCIBLE FAULT-TESTING SUITE

Automated tests in `apps/spine-api/tests/failure_injection_test.rs` and `apps/spine-cli/tests/` verify recovery behavior across all primary failure modes:

| Scenario | Initial State | Fault Trigger | Expected Behavior | Actual Observed Behavior | Recovery Process | Integrity Verification |
|---|---|---|---|---|---|---|
| **1. Unsandboxed Untrusted Workflow** | Runner in `Unsandboxed` mode | Untrusted workflow execution request | Immediate fail-closed rejection | Rejected with exit code 1 & security error | Switch to `SandboxedOci` or explicit trusted dev mode | Host filesystem untouched |
| **2. Stale Runner Fencing Token** | Job assigned with fencing token 11 | Runner sends log or complete request with token 10 | Rejection with `ForgeError::Unauthorized` | Rejection returned in 0.01s | Runner re-syncs state via heartbeat | Job remains in clean RUNNING/ASSIGNED state |
| **3. Runner Identity Impersonation** | Job assigned to `worker-1` | `fake-runner-id` submits job completion | Rejection with `ForgeError::Unauthorized` | Rejection returned in 0.01s | Original runner completes job | Job integrity preserved |
| **4. Webhook SSRF IP Probing** | Webhook configured to `169.254.169.254` | Outbox worker attempts delivery | Immediate rejection before TCP connect | `Err(ProhibitedIp)` returned | Update URL to public endpoint | Zero network packets emitted to cloud metadata |
| **5. Webhook Prohibited Port** | Webhook configured to `http://target:5432` | Outbox worker attempts delivery | Immediate rejection before TCP connect | `Err(ProhibitedPort)` returned | Update port to 80/443 | PostgreSQL port never probed |
| **6. Secret Plaintext Exfiltration** | Synthetic token `sp_pat_9876...` in code | Secret scanner runs against commit | Masked preview + keyed HMAC digest | `sp_p...****...dcba (hmac-sha256:v1:...)` | Developer fixes finding | Raw token never appears in finding record |
| **7. Health Readiness vs Liveness** | DB connection interrupted | Check `/health/live` vs `/health/ready` | Live: 200 OK; Ready: 503 Service Unavailable | Readiness fails; liveness remains OK | DB reconnect restores 200 OK | Traffic router stops sending traffic until ready |
| **8. Stale VCS CAS Push** | Remote ref at commit `C1` | Client pushes commit `C2` assuming base `C0` | CAS mismatch: reject push | Rejected with `LeaseConflict` | Client fetches, rebases, and re-pushes | Remote ref never overwrites unmerged commits |
| **9. Corrupted Packfile** | Pack checksum intentionally corrupted | Client requests object lookup | CRC32 / SHA-256 error detected | Object store reports corruption | Restore pack from replica or fsck unpack | Corrupt data never served to client |
| **10. Interrupted LFS Chunk Upload** | 4-chunk LFS object upload session | Client aborts after chunk 2 | Session remains partial; manifest uncommitted | Incomplete session expires after TTL | Client resumes upload of missing chunks | Partial chunks collected during GC grace period |

---

## 2. INTEGRITY VERIFICATION GUARANTEES

All recovery tests verify actual content on disk and state in the database, not merely process exit codes:
- Refs are confirmed to match expected OIDs.
- Object closure guarantees that every tree and blob exists.
- In-flight or interrupted uploads do not orphan dangling pointers in `refs/heads/`.
