# Spine Upgrade & Rollback Operational Procedures

## 1. Upgrade Principles & Safety Guarantees

Spine upgrades are designed to maintain backward compatibility and zero downtime for Git clone/fetch/push operations:
1. **Additive Schema Migrations:** All SQL migrations must be backward-compatible (new columns must be nullable or possess defaults). Deprecated columns are never dropped in the same release that introduces their replacements.
2. **Two-Phase Rolling Upgrades:**
   - Phase 1: Deploy new database schema migrations via `spine-api migrate`. The previous version of `spine-api` continues running safely.
   - Phase 2: Rolling update of `spine-api` instances behind the ingress load balancer.
3. **Pre-Upgrade Backup:** Always trigger a full coordinated backup before initiating an upgrade.

---

## 2. Standard Upgrade Workflow

```bash
# 1. Create safety backup
spine-api backup create --output /backups/pre_upgrade_backup.tar.gz

# 2. Pull new container image or install new binary
docker pull spine-forge/api:v0.2.0

# 3. Apply pending migrations
docker run --rm --net spine-net \
    -e DATABASE_URL="postgres://spine_app:password@postgres:5432/spine_forge" \
    spine-forge/api:v0.2.0 migrate

# 4. Perform rolling restart of API services
docker-compose up -d --no-deps spine-api

# 5. Verify readiness
curl -f http://127.0.0.1:3000/readyz
```

---

## 3. Rollback Procedure

If unexpected errors or performance degradation occur post-deployment:
1. **Revert Binary / Container Image:**
   ```bash
   docker-compose down spine-api
   # Point docker-compose.yml back to previous stable image tag (v0.1.0)
   docker-compose up -d spine-api
   ```
2. **Database Reversion (if required):**
   Because migrations are additive, reverting binaries does not break existing database schemas. If destructive schema corruption occurred, execute a full restore from the pre-upgrade backup:
   ```bash
   spine-api backup restore --input /backups/pre_upgrade_backup.tar.gz --target-storage /var/lib/spine/storage
   ```
