# Production Docker Compose Security Hardening

**File:** `docker-compose.yml`, `.env.example`  
**Milestone:** Batch 21 Remediation (Finding F-05)

---

## 1. Problem Statement (Finding F-05)

The previous `docker-compose.yml` relied on insecure defaults:
1. Hardcoded default passwords (`spine_prod_password`) in compose definitions.
2. Direct publishing of internal backend ports to the host machine (`5432` for Postgres, `8400` for Coordinator, `9101-9103` for Storage Nodes).
3. A single flat bridge network where internal storage nodes and public reverse proxies shared network namespace without segregation.

---

## 2. Hardened Architecture

### 2.1 Mandatory Environment Variable Validation
All credentials and encryption keys now use bash error substitutions:
```yaml
environment:
  POSTGRES_PASSWORD: ${POSTGRES_PASSWORD:?Error: POSTGRES_PASSWORD environment variable must be set}
  SPINE_JWT_SECRET: ${SPINE_JWT_SECRET:?Error: SPINE_JWT_SECRET environment variable must be set}
  SPINE_BACKUP_PASSPHRASE: ${SPINE_BACKUP_PASSPHRASE:?Error: SPINE_BACKUP_PASSPHRASE environment variable must be set}
```
Attempting to start the stack without defining these secrets fails immediately with a clear configuration error.

### 2.2 Network Segregation
The infrastructure is split into two non-overlapping bridge networks:
1. `spine-internal`: Dedicated solely to communication between PostgreSQL, Coordinator, Storage Nodes, and Spine API. Completely isolated from the internet and host network.
2. `spine-public`: Connects the Spine API to reverse proxies / TLS terminators.

### 2.3 Unmapped Internal Ports
Host port mappings for PostgreSQL (`5432`), Spine Coordinator (`8400`), and Storage Nodes (`9101-9103`) have been removed. Only the Spine API and SSH listener (`3000` and `2222`) or reverse proxy (`80`/`443`) are mapped to the host.
