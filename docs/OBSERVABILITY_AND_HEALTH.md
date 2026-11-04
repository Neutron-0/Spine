# OBSERVABILITY, HEALTH & DIAGNOSTICS ARCHITECTURE

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. HEALTH AND READINESS ENDPOINTS

Spine Forge exposes three dedicated health and diagnostic routes in `api/routes_health.rs`:

### 1. Liveness Probe: `GET /health/live` (and `/health`)
- **Purpose:** Kubernetes / load balancer process liveness check.
- **Behavior:** Returns HTTP 200 OK immediately if the HTTP server process is running.
- **Payload:**
```json
{
  "status": "ok",
  "service": "spine-forge-api",
  "version": "0.1.0"
}
```

### 2. Readiness Probe: `GET /health/ready`
- **Purpose:** Traffic readiness check. Determines whether the service can accept read/write requests.
- **Behavior:** Checks both PostgreSQL database connectivity and storage coordinator health.
  - Returns HTTP 200 OK if both are responsive.
  - Returns HTTP 503 Service Unavailable if either database or coordinator is down.
- **Payload:**
```json
{
  "status": "ready",
  "database": true,
  "coordinator": true
}
```

### 3. Structured Diagnostics: `GET /health/diagnostics`
- **Purpose:** Operational metrics and subsystem queue inspection.
- **Behavior:** Gathers bounded real-time diagnostics across runners, workflow queues, outbox, and repositories without leaking sensitive credentials.
- **Payload:**
```json
{
  "status": "HEALTHY",
  "service": "spine-forge-api",
  "version": "0.1.0",
  "database_connected": true,
  "coordinator_connected": true,
  "active_runners_count": 4,
  "pending_webhook_outbox_depth": 0,
  "queued_workflow_jobs_count": 0,
  "total_public_repositories": 12,
  "timestamp_nanos": 1730707200000000000
}
```

---

## 2. STRUCTURED TRACING & LOG PRIVACY

1. **Tracing Layer:** Axum routers are wrapped with `tower_http::trace::TraceLayer`.
2. **Credential Sanitization:**
   - Raw request bodies containing passwords, PATs, or webhook secrets are never logged at `INFO` or `WARN` levels.
   - Secret scanner outputs use keyed HMAC-SHA256 digests (`hmac-sha256:v1:...`).
   - Runner execution output masks all known repository secrets before logging.
