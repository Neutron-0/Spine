# ARTIFACT QUOTAS AND CAPACITY MANAGEMENT

## 1. Accounting Model

Spine maintains two distinct metrics for artifact capacity accounting:
1. **Logical Usage**: The sum of uncompressed sizes of all artifacts currently referenced by user/organization repositories, releases, generic artifacts, and packages.
2. **Physical Usage**: The actual disk footprint consumed by stored chunks after content-addressed deduplication.

When two repositories or release assets store identical 1 GB blobs, logical usage reflects 2 GB (accounting for ownership of two distinct assets), while physical disk usage remains only 1 GB.

---

## 2. Quota Enforcement

Quotas are configured per organization or individual user account:
- Default free tier: 10 GB limit (`max_bytes = 10_737_418_240`).
- Configurable per-tenant override via Forge administration.

### Ingestion Enforcement
1. **Batch/Upload Initialization**: When a client requests `POST /lfs/objects/batch` or uploads a release asset, `ArtifactService::check_quota` verifies:
   $$\text{used\_bytes} + \text{requested\_bytes} \le \text{max\_bytes}$$
   If exceeded, the server rejects the request with HTTP 403 Forbidden (`QUOTA_EXCEEDED`).
2. **Atomic Increment**: On session finalization or asset creation, `record_quota_consumption` transactionally increments the tenant's `used_bytes`.
3. **Reclaim on Deletion**: When releases, assets, or package versions are deleted or pruned, quota is adjusted accordingly.

---

## 3. Database Schema (`artifact_quotas`)

```sql
CREATE TABLE artifact_quotas (
    owner_id VARCHAR(64) PRIMARY KEY,
    owner_type VARCHAR(16) NOT NULL, -- 'USER' or 'ORGANIZATION'
    used_bytes BIGINT NOT NULL DEFAULT 0,
    max_bytes BIGINT NOT NULL DEFAULT 10737418240, -- 10 GiB
    updated_at_nanos BIGINT NOT NULL
);
```
