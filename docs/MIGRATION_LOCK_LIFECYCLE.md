# PostgreSQL Migration Lock Lifecycle & Cancellation Safety

**Module:** `spine_api::db::migrations`  
**Constant:** `SPINE_MIGRATION_ADVISORY_LOCK_ID = 0x5350_494E_454D_4947` (ASCII "SPINEMIG")  
**Verification Suite:** `tests/advisory_lock_test.rs`  
**Milestone:** Batch 21 Remediation (Finding F-07)

---

## 1. Finding F-07 Analysis

In earlier batches, migration serialization used the session-level advisory lock function:
```sql
SELECT pg_advisory_lock(0x5350494E454D4947);
```
Session-level advisory locks remain active until explicitly unlocked via `pg_advisory_unlock` or until the physical database connection closes.

### The Cancellation Flaw:
In asynchronous Rust (`tokio`), if an async task executing a migration is aborted, cancelled (via `tokio::select!` timeout), or panics:
1. Future execution stops before reaching `pg_advisory_unlock`.
2. The physical connection is returned to the `sqlx::PgPool` for reuse.
3. The advisory lock remains held inside the pooled connection!
4. Any other instance or replica attempting to boot and run migrations hangs indefinitely waiting for the lock.

---

## 2. Transaction-Level Advisory Lock Remediation

In Batch 21, the locking mechanism was refactored to use transaction-scoped advisory locks:
```rust
// 1. Acquire transaction
let mut tx = pool.begin().await.map_err(|e| {
    ForgeError::Internal(format!("Failed to begin migration transaction: {}", e))
})?;

// 2. Acquire transaction-level advisory lock
sqlx::query("SELECT pg_advisory_xact_lock($1)")
    .bind(SPINE_MIGRATION_ADVISORY_LOCK_ID)
    .execute(&mut *tx)
    .await
    .map_err(|e| {
        ForgeError::Internal(format!("Failed to acquire migration advisory lock: {}", e))
    })?;

// 3. Execute all migrations within this transaction
for migration in MIGRATIONS {
    // ...
}

// 4. Commit releases lock atomically
tx.commit().await.map_err(|e| {
    ForgeError::Internal(format!("Failed to commit migration transaction: {}", e))
})?;
```

---

## 3. Guarantees & Verification

`pg_advisory_xact_lock` guarantees:
1. **Auto-Release on Abort:** If the task is dropped or cancelled, `tx` is dropped, triggering an immediate database `ROLLBACK`. The PostgreSQL engine automatically frees the lock.
2. **Auto-Release on Crash:** If the API worker process terminates abruptly, the connection terminates, and PostgreSQL releases the lock.
3. **Tested Lifecycle:** Verified in `tests/advisory_lock_test.rs`, proving that dropping a transaction while holding the lock allows a concurrent transaction to acquire it within milliseconds without any deadlocks.
