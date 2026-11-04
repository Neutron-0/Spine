# Spine Production Observability & Monitoring Architecture

## 1. Monitoring Pillars

Spine observability is structured around:
1. **Health Probes (`/healthz`, `/readyz`, `/health/diagnostics`)**: Lightweight HTTP endpoints for Kubernetes probes and load balancers.
2. **Prometheus Metrics**: High-resolution latency, throughput, error rate, and storage utilization counters.
3. **Structured Tracing & Logs**: JSON-formatted log streams emitted via Tokio `tracing` and `tracing-subscriber`.

---

## 2. Health & Readiness Endpoints

### 2.1 Liveness Probe (`/healthz`)
- **Target:** Ingress and container orchestrator restart triggers.
- **Contract:** Returns HTTP 200 OK as long as the process event loop is healthy and responding.

### 2.2 Readiness Probe (`/readyz`)
- **Target:** Ingress traffic routing and blue/green deployment cutover.
- **Contract:** Verifies:
  - Database connectivity (SQL query `SELECT 1` or memory state healthy).
  - Storage directory is mounted and read/write capable.
  - Returns HTTP 200 OK if ready to accept traffic; HTTP 503 Service Unavailable if degraded.

### 2.3 Detailed Diagnostics (`/health/diagnostics`)
Returns complete system status:
```json
{
  "status": "healthy",
  "version": "0.1.0",
  "database": { "status": "connected" },
  "storage": { "status": "mounted", "path": "/var/lib/spine/storage" },
  "coordinator": { "status": "connected", "addr": "coordinator:8400" },
  "timestamp_nanos": 1793839200000000000
}
```

---

## 3. Key Performance Indicators & Alert Thresholds

| Metric Name | Description | Warning Threshold | Critical Threshold | Action Required |
|---|---|---|---|---|
| `spine_http_request_duration_seconds` | Git HTTP/REST API latency | P95 > 500ms | P99 > 2000ms | Inspect database locks or storage IOPS saturation |
| `spine_ssh_active_connections` | Concurrent SSH Git sessions | > 500 | > 900 | Scale API instances or increase file descriptor limits |
| `spine_storage_disk_used_percent` | Storage volume capacity | > 75% | > 85% | Trigger disk expansion or run artifact garbage collection |
| `spine_replication_lag_seconds` | Replica synchronization delay | > 5s | > 30s | Investigate network partition or degraded storage node |
| `spine_workflow_queue_depth` | Queued CI/CD jobs | > 50 | > 200 | Spin up additional isolated runner workers |
