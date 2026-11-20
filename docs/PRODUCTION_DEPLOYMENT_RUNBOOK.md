# Spine Forge — Production Deployment & Operations Runbook

**Document Identifier:** `SPINE-OPS-RUNBOOK-DEPLOY-01`  
**Classification:** Production Operations Standard Operating Procedure  
**Profile:** `spine-core-forge-v1`  
**Target Topology:** Docker Compose / Segregated Multi-Container Edge Ingress

---

## 1. Overview & Architecture

The Spine Forge production deployment (`spine-core-forge-v1`) orchestrates seven cooperating container services across segregated networks:

```text
                                [ Internet / Client Traffic ]
                                             |
                                  +---------------------+
                                  | 80/tcp, 443/tcp     | 2222/tcp (SSH)
                                  v                     v
                         +-----------------+    +-----------------+
                         | ingress (Nginx) |    | spine-api (SSH) |
                         +-----------------+    +-----------------+
                             /          \                |
                 /api/, /git/            Web UI          |
                            v              v             |
                    +---------------+  +----------------+ |
                    |   spine-api   |  | spine-frontend | |
                    +---------------+  +----------------+ |
                      /     |     \                       |
                     /      |      \----------------------+
                    v       v               v
            +----------+ +-------------+ +-----------------------------------+
            | postgres | | coordinator | | storage-node-1/2/3 (TripleCopy CAS)|
            +----------+ +-------------+ +-----------------------------------+
```

### 1.1 Network Segregation
* **`spine-public`:** Bridges external requests to `ingress` (ports `80` and `443`) and `spine-api` (SSH port `2222`).
* **`spine-ingress-proxy`:** Internal network routing reverse proxy traffic between `ingress`, `spine-frontend`, and `spine-api`.
* **`spine-internal`:** Strictly isolated backend network. PostgreSQL, cluster coordinator, storage nodes, and `spine-api` communicate here. Zero external port mappings.

---

## 2. Configuration & Certificate Reconciled Standards

All production parameters are configured via `.env` (derived from `.env.example`).

### 2.1 TLS Certificate Convention
Edge TLS ingress termination requires an X.509 certificate chain and matching private key:

| Configuration Variable | Default Host Path | Container Ingress Mount Point | Description |
| :--- | :--- | :--- | :--- |
| `SPINE_TLS_CERT_FILE` | `./deploy/nginx/certs/tls.crt` | `/etc/spine/certs/tls.crt:ro` | PEM-encoded certificate chain |
| `SPINE_TLS_KEY_FILE` | `./deploy/nginx/certs/tls.key` | `/etc/spine/certs/tls.key:ro` | PEM-encoded private key (`0600`) |

### 2.2 Mandatory Secrets Checklist
The Compose stack enforces non-empty variables with bash error substitutions:
* `POSTGRES_ADMIN_PASSWORD`: Minimum 16 characters (used for initial DB cluster bootstrap).
* `POSTGRES_MIGRATOR_PASSWORD`: Minimum 16 characters (DDL schema owner).
* `POSTGRES_APP_PASSWORD`: Minimum 16 characters (DML runtime application user).
* `SPINE_AUTH_SECRET_KEY`: Minimum 32 characters (HMAC key for RemoteAuth & tokens).
* `SPINE_LFS_SECRET`: Minimum 32 characters (Git LFS HMAC token key).
* `SPINE_BACKUP_PASSPHRASE` or `SPINE_BACKUP_PASSPHRASE_FILE`: PBKDF2/AES-256-GCM backup encryption key.

---

## 3. Mandatory Preflight Validation

Before starting or updating the production container stack, execute the automated preflight check.

### On Linux / POSIX:
```bash
./scripts/preflight_deploy.sh .env docker-compose.prod.yml
```

### On Windows PowerShell:
```powershell
.\scripts\preflight_deploy.ps1 -EnvFile .env -ComposeFile docker-compose.prod.yml
```

### Preflight Gates Verified:
1. **Environment File Presence:** Verifies `.env` exists and contains required variable definitions.
2. **Secret Entropy & Length:** Confirms all database and application secrets meet length thresholds ($32\text{ chars}$ for auth/LFS, $16\text{ chars}$ for passwords).
3. **Certificate Readability:** Confirms that `SPINE_TLS_CERT_FILE` and `SPINE_TLS_KEY_FILE` exist, are non-empty regular files, and are readable by the runtime user.
4. **Compose Syntax & Interpolation:** Executes `docker compose --env-file .env -f docker-compose.prod.yml config --quiet` to ensure zero missing substitution variables.

---

## 4. Production Startup Procedure

Once the preflight check reports `[PASS]`, start the service stack:

### Step 1: Launch Containers
```bash
docker compose --env-file .env -f docker-compose.prod.yml up -d
```

### Step 2: Observe Healthcheck Transitions
Monitor container health transitions until all services report `healthy`:
```bash
docker compose --env-file .env -f docker-compose.prod.yml ps
```

Expected readiness sequence:
1. `postgres`, `coordinator`, and `storage-node-1/2/3` reach `healthy` state.
2. `spine-api` completes schema migrations and connects to storage; `/health/ready` returns HTTP 200; container becomes `healthy`.
3. `spine-frontend` responds to HTTP probes and becomes `healthy`.
4. `ingress` starts and verifies upstream connectivity, reporting `healthy`.

### Step 3: Run Ingress & API Sanity Probes
```bash
# Verify HTTP to HTTPS 301 redirection
curl -I http://127.0.0.1/

# Verify HTTPS edge ingress and TLS handshake
curl -k -I https://127.0.0.1/

# Verify API readiness through edge proxy
curl -k -s https://127.0.0.1/api/v1/health/ready | jq .
```

---

## 5. Out-of-Band Backup & Disaster Recovery

For production deployments where storage nodes exceed $15\text{ MiB}$ or complete multi-volume atomic recovery is required, follow the certified Out-of-Band Backup Runbook (`docs/OUT_OF_BAND_STORAGE_BACKUP_RUNBOOK.md`).

### Automated Backup Command:
```bash
export SPINE_BACKUP_REQUIRE_REMOTE=1
export SPINE_BACKUP_OBJECT_LOCK_MODE=COMPLIANCE
export SPINE_BACKUP_RETENTION_DAYS=90
export SPINE_BACKUP_REMOTE_S3_URI="s3://my-prod-backup-bucket/spine"
export SPINE_BACKUP_PASSPHRASE_FILE="/etc/spine/backup_key.secret"

./scripts/backup_oob.sh
```

### Automated Restore Command:
```bash
export SPINE_RESTORE_PASSPHRASE_FILE="/etc/spine/backup_key.secret"

./scripts/restore_oob.sh s3://my-prod-backup-bucket/spine/spine_backup_<timestamp>.spnebak
```

---

## 6. Incident Response & Troubleshooting

* **Ingress 502 Bad Gateway:** Check if `spine-api` is healthy (`docker compose -f docker-compose.prod.yml ps spine-api`). If unhealthy, check startup migration logs:
  ```bash
  docker compose -f docker-compose.prod.yml logs spine-api
  ```
* **Storage Replica Degraded:** Inspect coordinator and storage node connectivity:
  ```bash
  curl -k -s -H "Authorization: Bearer <TOKEN>" https://127.0.0.1/api/v1/repos/<owner>/<repo>/durability | jq .
  ```
* **Maintenance Barrier Stuck:** If a previous backup was interrupted before releasing the barrier:
  ```bash
  docker compose -f docker-compose.prod.yml exec -T postgres psql -U spine_admin -d spine_forge -c "DELETE FROM _cluster_maintenance_barrier;"
  ```
