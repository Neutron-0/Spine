# Spine Operations Manual & Daily Maintenance Guide

## 1. Routine Operational Checklist

### Daily:
- Check cluster health diagnostics: `curl -s http://127.0.0.1:3000/health/diagnostics | jq .`
- Review failed webhook delivery queue: `SELECT count(*) FROM webhook_deliveries WHERE status = 'failed' AND attempts >= 5;`
- Confirm overnight backup archive creation and SHA-256 verification log.

### Weekly:
- Verify available storage capacity on all storage nodes.
- Inspect orphan artifact and LFS objects eligible for garbage collection.
- Test disaster recovery restoration into an isolated staging environment.

### Monthly:
- Review security audit logs (`audit_events`) for unusual administrative access or failed login spikes.
- Update base container operating system patches (`apt-get update && apt-get upgrade`).

---

## 2. Common Administrative Operations

### One-Time Admin Account Initialization
```bash
spine-api bootstrap-admin --username spineadmin --email admin@example.com
```

### Manual Trigger of Database Migrations
```bash
spine-api migrate --database-url "postgres://spine_app:secret@postgres:5432/spine_forge"
```

### Performing an On-Demand Backup
```bash
spine-api backup create \
    --output /var/lib/spine/backups/manual_backup_$(date +%Y%m%d_%H%M%S).tar.gz \
    --storage-root /var/lib/spine/storage
```

### Restoring from a Backup
```bash
spine-api backup restore \
    --input /var/lib/spine/backups/manual_backup_20261105.tar.gz \
    --target-storage /var/lib/spine/storage
```
