# Production Database Startup and Health Verification

**Module:** `spine_api::main`, `spine_api::api::routes_health`  
**Milestone:** Batch 21 Remediation (Finding F-06)

---

## 1. Problem Statement (Finding F-06)

In previous builds, if `database_url` was omitted or unresolvable, the API server would silently fall back to initializing an in-memory database mock (`InMemoryDatabase`). This caused nodes in high-availability clusters to diverge silently, losing persistent state upon container restart without logging fatal errors.

---

## 2. Hardened Boot Sequence

### 2.1 Profile-Enforced Boot Requirements
In `apps/spine-api/src/main.rs`:
```rust
if profile.requires_persistent_database() {
    if in_memory_db {
        anyhow::bail!(
            "Deployment profile '{}' strictly prohibits '--in-memory-db'. A persistent PostgreSQL database is required.",
            profile
        );
    }
    let db_url = database_url
        .or_else(|| std::env::var("DATABASE_URL").ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "Mandatory database URL missing under profile '{}'. Provide --database-url or set DATABASE_URL.",
                profile
            )
        })?;
    
    let pg_db = PgDatabase::connect_and_migrate(&db_url).await
        .map_err(|e| anyhow::anyhow!("Failed to connect to or migrate PostgreSQL: {}", e))?;

    // Immediate ping verification
    pg_db.ping().await
        .map_err(|e| anyhow::anyhow!("PostgreSQL database liveness check failed immediately after connect: {}", e))?;

    Arc::new(pg_db)
}
```

### 2.2 Health Check Route Verification (`/health/ready`)
In `apps/spine-api/src/api/routes_health.rs`:
- `/health/ready` executes `state.db.ping().await`.
- If the database is connected and responsive, it returns `200 OK`.
- If the ping fails or times out, it returns `503 Service Unavailable` with `status: "not_ready"`, enabling container orchestrators (Kubernetes, Nomad, Docker Compose) to fail health probes and take corrective action.
