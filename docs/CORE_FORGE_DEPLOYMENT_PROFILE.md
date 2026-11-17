# Core Forge Deployment Profile Specification

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Profile Identifier:** `spine-core-forge-v1`  
**Security Boundary:** High-Trust Control Plane & Authoritative Repository Storage (Zero Untrusted Code Execution)

---

## 1. Architectural Philosophy and Threat Model

The Core Forge Deployment Profile encompasses all authoritative services responsible for user authentication, repository hosting, Git protocol endpoints (HTTPS Smart HTTP and native SSH), cluster coordination, and PostgreSQL metadata.

### Strict Non-Execution Rule:
**Untrusted CI workflows, build scripts, tests, or arbitrary user code MUST NEVER execute on Core Forge instances.**

Allowing untrusted workflow scripts to execute on the same hosts that manage encryption keys, database connections, and authoritative repository files introduces critical risks of privilege escalation, lateral network movement, and data exfiltration.

All workflow execution is offloaded to remote, segregated runners under the `spine-ci-runner-v1` deployment profile.

---

## 2. Core Service Topology

```text
               +-------------------------------------------+
               |          External Reverse Proxy           |
               |          (NGINX / HAProxy / Envoy)        |
               +-------------------------------------------+
                       |                           |
        HTTPS: 443 / HTTP: 3000             SSH: 22 / 2222
                       |                           |
                       v                           v
               +-------------------------------------------+
               |               spine-api                   |
               |  (HTTP API, Web UI, Git Smart HTTP, SSH)  |
               +-------------------------------------------+
                       |                           |
                       | SQL Connections           | Raft / Cluster RPC
                       v                           v
        +----------------------------+   +----------------------------+
        |     PostgreSQL 18 Cluster  |   |     spine-coordinator      |
        |  (spine_forge database)    |   |  (Leader Election / Leases)|
        +----------------------------+   +----------------------------+
                                                   |
                                                   | Object Replication
                                                   v
                                         +----------------------------+
                                         |     spine-storage-node     |
                                         | (Bare Repos, Packfiles)    |
                                         +----------------------------+
```

---

## 3. Configuration & Environment Variables

Core Forge instances are configured via `/etc/spine/spine-api.env`:

```bash
# General Instance Identity
SPINE_NODE_ID=core-api-01
SPINE_ENVIRONMENT=production
SPINE_LOG_LEVEL=info,spine_api=debug

# Network Binding
SPINE_HTTP_BIND=0.0.0.0
SPINE_HTTP_PORT=3000
SPINE_SSH_BIND=0.0.0.0
SPINE_SSH_PORT=2222
SPINE_BASE_URL=https://forge.internal.net

# Database Connectivity (PostgreSQL 18+)
SPINE_DATABASE_URL=postgres://spine_app:StrongSecretPassword@postgres.internal.net:5432/spine_forge?sslmode=verify-full
SPINE_DB_MAX_CONNECTIONS=50
SPINE_DB_MIN_CONNECTIONS=10
SPINE_DB_TIMEOUT_SECONDS=30

# Authoritative Storage
SPINE_STORAGE_ROOT=/var/lib/spine/storage
SPINE_SSH_HOST_KEY=/var/lib/spine/ssh/host_key_ed25519

# Cluster Coordination
SPINE_COORDINATOR_ADDRS=10.0.1.10:8400,10.0.1.11:8400,10.0.1.12:8400
SPINE_DURABILITY_POLICY=TripleCopy

# Security Controls
SPINE_CI_LOCAL_RUNNER_ENABLED=false
SPINE_ALLOW_REGISTRATION=false
SPINE_SESSION_TIMEOUT_SECONDS=86400
SPINE_BACKUP_PASSPHRASE_FILE=/etc/spine/backup_key.secret
```

---

## 4. Systemd Sandboxing Verification

Core Forge services run under unprivileged user `spine:spine` with OS sandboxing:
- Strict read-only root filesystem (`ProtectSystem=strict`).
- Total isolation of home directories (`ProtectHome=true`).
- Blocked elevation (`NoNewPrivileges=true`).
- Blocked device node creation (`PrivateDevices=true`).
- Confined write permissions: strictly restricted to `/var/lib/spine`.

---

## 5. Disaster Recovery & Storage Backup Procedures

- **In-Band Cluster RPC Backups (`SPNEBAK v2`):** Supported for participant volumes up to `15 MiB` (`15,728,640` bytes) compressed payload per node (`MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES`), protected by authenticated AES-256-GCM encryption and RAII writer-draining barriers.
- **Out-of-Band Volume Snapshots ($> 15\text{ MiB}$):** For production clusters exceeding the 15 MiB single-frame cluster RPC limit, operators must follow the documented, validated out-of-band operational runbook at [OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md](OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md) to perform atomic cloud block storage or filesystem snapshots and fail-closed post-restore verification.
