# Spine — Batch 19 Deployment Blocker Register

**Target Milestone:** Batch 19  
**Workspace:** `d:\Vault\Spine`  
**Current Baseline Commit:** `b6881c6`  
**Date of Audit:** 2026-11-05  

---

## 1. Blocker Inventory & Classification Matrix

| Blocker ID | Classification | Subsystem | Description & Evidence | Severity | Remediation Action & Status |
|---|---|---|---|---|---|
| **BLK-01** | **Critical Database** | PostgreSQL Migrations | No versioned `.sql` migration files exist in `github-alternative/backend/` despite 5,500+ lines of queries in `postgres.rs`. Fresh database setup fails without manual schema creation. | **CRITICAL** | Author 11 sequential migration scripts (`0001` through `0011`) covering all 60+ tables, indices, and foreign keys. Wire embedded migration runner in `apps/spine-api`. |
| **BLK-02** | **Critical Database** | PostgreSQL Runtime Verification | Live PostgreSQL daemon is absent in local Windows test environment; integration tests ran against `InMemoryDatabase`. Live database behavior must be explicitly verified or marked BLOCKED. | **CRITICAL** | Build complete reproducible PostgreSQL profile, Docker Compose dev profile, and standalone integration test harness. Explicitly document daemon status. |
| **BLK-03** | **Critical Security** | Untrusted CI Runner Execution | Running arbitrary untrusted user CI workflows on bare metal Windows or container host without VM boundary is a severe security risk. | **CRITICAL** | Explicitly define **Core Forge Profile** with untrusted runner execution disabled by default. Require VM boundary for isolated runner profile. |
| **BLK-04** | **Critical Data Integrity** | Backup & Restore Pipeline | No automated, coordinated cross-system backup mechanism exists linking PostgreSQL metadata, native `.spine/` repositories, LFS chunks, and release assets. | **CRITICAL** | Implement `spine-api backup create` and `restore` with cryptographic manifest, SHA-256 verification, and execute full restoration drill. |
| **BLK-05** | **Critical Infrastructure** | Single Point of Failure (Coordinator) | Storage coordinator is single-instance; loss of coordinator disrupts replication and write leases. | **HIGH (Reliability)** | Document single-instance pilot limitation; define operational recovery runbook and data durability guarantees. |
| **BLK-06** | **Operational** | Service Health & Readiness Probes | API process listening does not differentiate process liveness from database readiness or storage mount availability. | **MEDIUM** | Implement explicit `/healthz` (liveness) and `/readyz` (readiness probing DB pool, storage directory, and SSH daemon). |
| **BLK-07** | **Operational** | Administrative Bootstrap | No secure, one-time initial administrator bootstrap mechanism; default dev accounts cannot be used in production. | **HIGH** | Implement `spine-api bootstrap-admin` CLI command with one-time token generation, preventing re-execution. |
| **BLK-08** | **Operational** | Production Service Packaging | Missing production multi-stage Dockerfile, systemd service units, and `.env.example` deployment configuration. | **MEDIUM** | Author multi-stage container builds, docker-compose orchestration, systemd service definitions, and security configs. |
| **BLK-09** | **Performance & Economics** | Large-Storage Capacity & Quotas | Hosting > 200 GB code with multi-copy replication requires at least 400–600 GB raw storage before backups. | **MEDIUM** | Produce comprehensive capacity, durability, and cost models (`docs/DEPLOYMENT_CAPACITY_AND_COST_MODEL.md`). |

---

## 2. Detailed Findings and Action Plans

### BLK-01 & BLK-02: PostgreSQL Migrations and Execution Gate
- **Evidence:** `apps/spine-api/src/db/postgres.rs` contains thousands of lines of queries expecting tables (`users`, `forge_repositories`, `lfs_locks`, etc.), but `github-alternative/backend/` lacked an `apps/spine-api/migrations` folder with SQL DDL scripts.
- **Action:**
  1. Author idempotent, ordered SQL migration files `0001_core_identity.sql` to `0011_security_center.sql` in `apps/spine-api/migrations/`.
  2. Implement `run_migrations(&pool)` using `sqlx::migrate!` or an embedded runner.
  3. Wire migration runner into `main.rs` and provide a dedicated `migrate` CLI subcommand.
  4. Create a dedicated PostgreSQL integration test harness that connects to `DATABASE_URL` when available and gracefully documents the environment status when absent.

### BLK-03: Untrusted CI Runner Execution
- **Evidence:** `IsolatedRunnerExecutor` isolates directory trees on the local host, which is suitable for trusted self-hosted runners but insufficient for hostile public multi-tenant workloads.
- **Action:**
  1. Formally specify the **Core Forge Profile** where untrusted runner registration is disabled in the UI and rejected by the API.
  2. Document the **Isolated CI Profile** requiring hypervisor-level microVM isolation (e.g. Firecracker, Cloud Hypervisor) for untrusted workloads.

### BLK-04: Coordinated Backup and Disaster Recovery
- **Evidence:** While `.spine/` repositories and `spine-artifact` chunk stores exist on disk, no unified tool existed to snapshot the database, repository packfiles, and artifact stores synchronously into a verifiable backup package.
- **Action:**
  1. Implement automated backup creation utility in `spine-api` generating a timestamped tar/zip package with `backup_manifest.json` containing SHA-256 hashes of all components.
  2. Implement automated restoration utility validating manifest hashes and rehydrating database and storage roots into clean target directories.
  3. Perform an end-to-end restore drill and measure recovery times.

### BLK-06 & BLK-07: Health Endpoints and Admin Bootstrap
- **Evidence:** `apps/spine-api` lacked `/readyz` probing database and storage mounts, and relied on dev seeding for users.
- **Action:**
  1. Add `/healthz` (liveness) and `/readyz` (readiness) routes in `api/routes_health.rs`.
  2. Add `bootstrap-admin` CLI command in `main.rs` that generates a secure randomized password or accepts a one-time token, sets `account_status: "ACTIVE"`, and flags initialization complete.

### BLK-08: Production Service Packaging
- **Evidence:** The repository lacked production Dockerfile, systemd service units, and documented deployment compose files.
- **Action:**
  1. Create multi-stage `Dockerfile` targeting minimal Debian/Alpine runtime with unprivileged user `spine`.
  2. Create `docker-compose.yml` for single-node pilot and `docker-compose.dev.yml` for local development.
  3. Create systemd unit files: `spine-api.service`, `spine-coordinator.service`, `spine-storage-node.service`.
  4. Create `.env.example` documenting all configuration keys.
