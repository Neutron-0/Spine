# Spine Incident Response & Emergency Operations Runbook

## 1. Incident Classification Matrix

- **SEV-1 (Critical Outage):** Complete inability for developers to push or clone via HTTP and SSH, or potential data loss detected.
- **SEV-2 (Major Degradation):** High latency (> 5s) on Git operations, CI/CD runner execution paused, or one storage replica zone down.
- **SEV-3 (Minor Disruption):** Web UI search slow, discussion comments lagging, non-critical webhooks failing delivery.

---

## 2. Emergency Operational Playbooks

### Playbook 1: Total Git Push Rejection (SSH & HTTP Failing)
1. **Check Process & Probe Status:**
   ```bash
   curl -i http://127.0.0.1:3000/readyz
   systemctl status spine-api
   ```
2. **Inspect Write Storage & Disk Space:**
   ```bash
   df -h /var/lib/spine/storage
   ```
   If disk usage is 100%, immediately allocate emergency volume capacity or clean up temporary files in `/var/lib/spine/storage/runner_workspaces/`.
3. **Inspect Storage Node & Coordinator Connectivity:**
   ```bash
   curl http://127.0.0.1:3000/health/diagnostics
   ```
   Check if coordinator lease epochs have expired or if a split-brain condition is preventing quorum.

### Playbook 2: PostgreSQL Connection Exhaustion
1. **Symptom:** `spine-api` logs show `pool timed out waiting for an open connection` or HTTP 500 errors across web routes.
2. **Inspect PostgreSQL Active Connections:**
   ```sql
   SELECT count(*), state FROM pg_stat_activity GROUP BY state;
   ```
3. **Remediation:**
   - Increase PostgreSQL `max_connections` (e.g. from 100 to 300 in `postgresql.conf`).
   - Ensure PgBouncer or native connection pooling is enabled.
   - Terminate hanging idle-in-transaction queries:
     ```sql
     SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE state = 'idle in transaction' AND state_change < now() - INTERVAL '5 minutes';
     ```

### Playbook 3: SSH Host Key Identification Changed Error
1. **Symptom:** Client reports `WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! Host key verification failed`.
2. **Diagnosis:** Server host key was overwritten or recreated with a new key instead of retaining the persistent key.
3. **Remediation:**
   - Restore `ssh/host_key_ed25519` from backup archive.
   - Verify permissions: `chmod 600 /var/lib/spine/storage/ssh/host_key_ed25519`.
   - Restart `spine-api`.
