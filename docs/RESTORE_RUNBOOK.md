# Spine Restoration & Recovery Runbook

## 1. Prerequisites

Before initiating restoration:
- Ensure `spine-api` service is stopped on the target host:
  ```bash
  systemctl stop spine-api
  ```
- Verify available disk capacity in target directory (must be at least 2x the uncompressed size of the backup archive).
- Locate the target verified backup archive: `/backups/spine_backup_<timestamp>.tar.gz`.

---

## 2. Step-by-Step Restoration Procedure

### Step 1: Execute Automated Storage & Manifest Restore
Run the built-in restoration command:
```bash
spine-api backup restore \
    --input /backups/spine_backup_20261105.tar.gz \
    --target-storage /var/lib/spine/storage
```
**Expected Output:**
```
Restoring backup archive from /backups/spine_backup_20261105.tar.gz into /var/lib/spine/storage
Backup restoration completed:
  Backup ID:     7f9c2d1b-8e34-4b9a-a82f-0b2a8d4a9f31
  Files restored:4512
  Bytes restored:104857600
  Integrity:     VERIFIED (SHA-256 matched)
```

### Step 2: Restore Database Metadata (PostgreSQL)
If restoring into a fresh PostgreSQL instance:
1. Ensure the database exists:
   ```bash
   createdb -U spine_app -h 127.0.0.1 spine_forge
   ```
2. Run database migrations to establish target schema:
   ```bash
   spine-api migrate --database-url "postgres://spine_app:password@127.0.0.1:5432/spine_forge"
   ```
3. Load database dump or JSON snapshot if separate dump utility used:
   ```bash
   pg_restore -U spine_app -d spine_forge /backups/postgres_dump.dump
   ```

### Step 3: Validate File Ownership & Permissions
Ensure all restored files are owned by the unprivileged `spine` user:
```bash
chown -R spine:spine /var/lib/spine
chmod 700 /var/lib/spine/storage/ssh
chmod 600 /var/lib/spine/storage/ssh/host_key_ed25519
chmod 644 /var/lib/spine/storage/ssh/host_key_ed25519.pub
```

### Step 4: Restart Services & Verify Health
1. Start Spine API:
   ```bash
   systemctl start spine-api
   ```
2. Probe liveness and readiness endpoints:
   ```bash
   curl -f http://127.0.0.1:3000/healthz
   curl -f http://127.0.0.1:3000/readyz
   curl -f http://127.0.0.1:3000/health/diagnostics
   ```
3. Perform end-to-end smoke test:
   ```bash
   git clone ssh://git@127.0.0.1:2222/org/repo.git /tmp/smoke_test
   ```
