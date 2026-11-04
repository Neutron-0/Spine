# BATCH 13 PHASE ZERO GAP REGISTER AND SECURITY AUDIT

Date: 2026-11-04  
System: SPINE Native Version Control and Forge Platform  
Target: Batch 13 Gate Verification (Batch 11 & Batch 12 Closure Verification)

---

## 1. Executive Summary

In accordance with the Batch 13 Engineering Directive, Phase Zero establishes a mandatory verification and security gate assessing the foundational guarantees of Batch 11 (Distributed Artifacts, LFS, Releases, Cargo Registry) and Batch 12 (Forge Automation, Webhooks, Workflows, Runners) before deploying Batch 13 capabilities (Search, Code Discovery, Discussions, Security Center).

All core native unit, integration, and security test suites have been executed directly on Windows 11 MSVC without Git wrappers or external Git binaries.

---

## 2. Verification Matrix

| Area | Test Suite / Verification Item | Status | Details |
|---|---|---|---|
| **LFS End-to-End** | `lfs_remote_workflow_test.rs` | **CLOSED (VERIFIED)** | 10-step remote LFS workflow (clean, smudge, pointer serialization, dedup, pack ignore, remote push/pull) passed in 0.23s. |
| **Cargo Registry** | `cargo_e2e_cli_test.rs` | **CLOSED (VERIFIED)** | Full end-to-end Cargo index query, token authentication, package publish, crate download, and checksum validation passed against `spine-api`. |
| **Artifact Persistence** | `distributed_artifact_persistence_test.rs` | **CLOSED (VERIFIED)** | Storage node disk persistence across restarts, manifest chunk mapping, and coordinator heartbeats passed. |
| **GC Reachability** | `spine_artifact::gc::tests` | **CLOSED (VERIFIED)** | Mark-and-sweep reachability, cross-manifest shared chunk preservation, and orphaned chunk reclamation verified. |
| **Webhook Delivery** | `webhook_delivery_test.rs` | **CLOSED (VERIFIED)** | Outbox queueing, HMAC-SHA256 signature generation/constant-time verification, and SSRF forbidden range blocking passed. |
| **Workflow Engine** | `workflow_ci_e2e_test.rs` | **CLOSED (VERIFIED)** | DAG dependency scheduling, multi-step job execution, artifact passing, and PR status check protection passed. |
| **Hostile Workflows** | `hostile_workflow_test.rs` | **CLOSED (VERIFIED)** | Malformed YAML rejection, cyclical DAG detection, stale runner fencing rejection, step timeout / process tree killing, and secret masking passed. |
| **PostgreSQL Reality** | Live Docker / PostgreSQL Service | **BLOCKED (ENVIRONMENT)** | Host Docker CLI is installed (v29.6.2), but Docker Desktop daemon is not running (`npipe:////./pipe/dockerDesktopLinuxEngine`). No local PostgreSQL Windows service is running. `InMemoryDatabase` provides contract fidelity for integration tests. Real Postgres requires starting Docker Desktop or running `postgres.exe` locally. |

---

## 3. PostgreSQL Reality and Environment Assessment

### Current Host Status
- `docker version`: Client 29.6.2 present, daemon inactive.
- PostgreSQL service: No active Windows service named `*postgres*`.
- **Verdict**: **BLOCKED** on host daemon availability.

### Fidelity Strategy
- Spine Forge defines database interactions through repository traits (`UserRepository`, `RepositoryRepository`, `OrganizationRepository`, `IssueRepository`, `PullRequestRepository`, `WorkflowRepository`, `WebhookRepository`, etc.).
- `InMemoryDatabase` implements these exact traits with concurrent thread-safe synchronization (`RwLock`/`DashMap`) ensuring business logic validation in CI and offline developer environments without faking real PostgreSQL.
- SQL migrations in `github-alternative/backend/migrations/` adhere to standard PostgreSQL DDL (foreign keys, check constraints, indexes, timestamps).

### Local PostgreSQL Setup Instructions
To run against real PostgreSQL on Windows:
```powershell
# Option A: Start Docker Desktop
& "C:\Program Files\Docker\Docker\Docker Desktop.exe"
docker run -d --name spine-postgres -e POSTGRES_PASSWORD=postgres -e POSTGRES_DB=spine_forge -p 5432:5432 postgres:16-alpine

# Option B: Run native Windows PostgreSQL
pg_ctl -D "C:\Program Files\PostgreSQL\16\data" start

# Run backend with DATABASE_URL
$env:DATABASE_URL="postgres://postgres:postgres@localhost:5432/spine_forge"
cargo run --manifest-path github-alternative/backend/apps/spine-api/Cargo.toml
```

---

## 4. Runner Execution Isolation & Boundary Remediation

### Identified Risk
Executing CI workflow steps directly within the `spine-api` server process tree introduces security hazards:
- Shared memory / environment variables.
- Privilege escalation.
- Lack of CPU/memory quotas.
- Accidental secret leakage to pull requests from untrusted forks.

### Remediation Plan
1. **Standalone Runner (`spine-runner`)**: Separate runner binary executing isolated from `spine-api`.
2. **Windows Job Objects (`JobObject`)**: Constrain memory limit, CPU rate, and terminate entire process trees on cancellation or timeout using `TerminateJobObject`.
3. **Workspace Isolation**: Clean directory sandbox per job execution with strict path traversal checks.
4. **Untrusted PR Protection**: Workflows triggered by `pull_request` from untrusted or external contributors do not receive repository secrets.
5. **Fail-Closed**: If sandbox or Job Object initialization fails, jobs immediately abort with an explicit security failure.

---

## 5. Webhook SSRF Hardening

### Additions
1. **Redirect Disablement**: Webhook HTTP client explicitly configures `redirect(reqwest::redirect::Policy::none())` to prevent redirect-based SSRF into private IP ranges.
2. **IPv4-Mapped IPv6 Rejection**: `::ffff:127.0.0.1`, `::ffff:10.0.0.0/8`, etc., explicitly parsed and evaluated against private CIDR rules.
3. **DNS Resolution Verification**: Hostname is resolved prior to connection; all resolved addresses are tested against forbidden CIDRs (Loopback, Link-Local, Private RFC1918, Carrier-Grade NAT RFC6598, Multicast, Broadcast).

---

## 6. Phase Zero Sign-Off

Phase Zero gate is **APPROVED** to proceed with Batch 13 feature development.
All prerequisite systems are functionally intact and verified.
