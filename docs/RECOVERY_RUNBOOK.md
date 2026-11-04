# SPINE DISASTER RECOVERY & INCIDENT RUNBOOK

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. INCIDENT: DATABASE DISCONNECTION OR POOL EXHAUSTION

### Symptoms
- `/health/ready` returns HTTP 503 `{"status":"not_ready","database":false}`.
- API requests fail with `DatabaseError` or acquire timeouts.

### Remediation Steps
1. Verify PostgreSQL service status on host:
   ```powershell
   Get-Service postgresql* | Start-Service
   ```
2. Check connection saturation:
   ```sql
   SELECT count(*), state FROM pg_stat_activity WHERE datname = 'spine_forge' GROUP BY state;
   ```
3. If connections are orphaned by an aborted test or killed worker, restart the `spine-api` process to drain and reinitialize the SQLx connection pool.
4. Verify `/health/ready` returns HTTP 200 OK.

---

## 2. INCIDENT: CORRUPTED OBJECT / PACKFILE DETECTED

### Symptoms
- `spine fsck` reports checksum mismatch or missing object closure.
- SPWP fetch or push fails with `CorruptedObject` error.

### Remediation Steps
1. Run repository fsck diagnostics:
   ```powershell
   spine fsck --verbose
   ```
2. If a packfile is corrupted, check if a valid replica exists on secondary storage nodes via coordinator:
   ```powershell
   spine-admin replica status --repo <repo_id>
   ```
3. Trigger anti-entropy repair:
   ```powershell
   spine-admin repair --repo <repo_id> --target-node <node_id>
   ```
4. Verify integrity post-repair:
   ```powershell
   spine fsck
   ```

---

## 3. INCIDENT: STUCK WORKFLOW JOB OR UNRESPONSIVE RUNNER

### Symptoms
- Workflow job remains in `RUNNING` status indefinitely.
- Dependent DAG jobs are blocked.

### Remediation Steps
1. The coordinator and API automatically invalidate job leases after 5 minutes (`lease_expires_at_nanos`).
2. Once the lease expires, the scheduler resets the job status to `QUEUED` and increments `fencing_token`.
3. If the dead runner subsequently attempts to report completion, its stale fencing token is rejected with `ForgeError::Unauthorized`.
4. Any active runner matching labels can then re-acquire the job and execute it cleanly.

---

## 4. INCIDENT: WEBHOOK OUTBOX ACCUMULATION (DELIVERY STALL)

### Symptoms
- `/health/diagnostics` reports `pending_webhook_outbox_depth > 1000`.
- Subscribers report delayed event notifications.

### Remediation Steps
1. Check if an external subscriber endpoint is timing out or returning HTTP 5xx.
2. Invalidate failing subscriptions or let exponential backoff exhaust `max_attempts` (5 attempts):
   - Items with exhausted attempts transition to status `FAILED` and cease retrying.
3. Restart outbox background worker to resume processing healthy pending events.
