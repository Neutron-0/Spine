# Backup Consistency Barrier Specification

**Module:** `spine_api::backup::consistency`  
**Components:** `ConsistencyBarrier`, `MaintenanceGuard`  
**Guarded Pipelines:** Git Smart HTTP `git-receive-pack`, Git LFS upload, Native Git-over-SSH `receive-pack`  
**Milestone:** Batch 21 Remediation (Finding F-01 / F-02)

---

## 1. Problem Statement

Creating point-in-time backups across distributed storage nodes and PostgreSQL while active repository pushes, ref updates, or LFS chunk uploads are in-flight risks capturing inconsistent partial states (e.g. ref points to a commit whose objects were written right after the tarball snapshot was taken).

---

## 2. Architecture & Design

The `ConsistencyBarrier` coordinates backup maintenance windows by pausing or draining in-flight writes before snapshots begin:

1. **Atomic State Tracking:**
   `ConsistencyBarrier` tracks state with an `AtomicBool` (`in_maintenance`), shared across the HTTP router, SSH server, and backup workers.
2. **Maintenance Mode Window:**
   When entering maintenance:
   ```rust
   let guard = barrier.enter_maintenance(Duration::from_millis(50)).await;
   ```
   Active write requests settle, and any incoming writes are rejected with HTTP 503 and a `Retry-After: 5` header.
3. **RAII Deactivation:**
   The `MaintenanceGuard` implements `Drop`. When backup completion (or failure) drops the guard, `in_maintenance` is automatically reset to `false`.

---

## 3. Route Guard Implementations

### Git Smart HTTP (`routes_git_http.rs`)
```rust
if state.consistency_barrier.is_maintenance_active() {
    return (
        StatusCode::SERVICE_UNAVAILABLE,
        [(axum::http::header::RETRY_AFTER, "5")],
        "Maintenance in progress. Backups active, repository writes temporarily drained.\n",
    ).into_response();
}
```

### Git LFS Upload (`routes_lfs.rs`)
```rust
if state.consistency_barrier.is_maintenance_active() {
    return (
        StatusCode::SERVICE_UNAVAILABLE,
        [(axum::http::header::RETRY_AFTER, "5")],
        "Maintenance in progress. Backups active, object uploads temporarily drained.\n",
    ).into_response();
}
```

### Git-over-SSH Receive-Pack (`ssh/server.rs`)
```rust
if self.state.consistency_barrier.is_maintenance_active() {
    let _ = session.channel_success(channel);
    let _ = session.extended_data(
        channel,
        1,
        Bytes::from_static(b"FATAL: Maintenance in progress. Backups active, repository writes temporarily drained.\n"),
    );
    let _ = session.exit_status_request(channel, 1);
    let _ = session.close(channel);
    return Ok(());
}
```
