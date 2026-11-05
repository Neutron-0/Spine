# PostgreSQL Least Privilege Security Profile

**Database User:** `spine_app`  
**Application Database:** `spine_forge`  
**Configuration Script:** `scripts/setup_postgres_wsl.sh`  
**Verification Suite:** `tests/postgres_least_privilege_test.rs`  
**Milestone:** Batch 21 Remediation (Finding F-04)

---

## 1. Finding F-04 Summary

The first Codex deployment audit found that the application database role `spine_app` held `SUPERUSER` and `CREATEDB` privileges, and PostgreSQL was listening on `0.0.0.0/0` in development and staging environments.

If SQL injection or an arbitrary query vulnerability occurred in the application, full database cluster takeover or unauthorized superuser operations (e.g. `COPY ... FROM PROGRAM`) were possible.

---

## 2. Least Privilege Security Hardening

The application user `spine_app` was reconfigured with zero administrative capabilities:

```sql
-- Role hardening
ALTER ROLE spine_app WITH NOSUPERUSER NOCREATEDB NOCREATEROLE NOBYPASSRLS;

-- Ownership & schema permissions
GRANT CONNECT ON DATABASE spine_forge TO spine_app;
GRANT USAGE, CREATE ON SCHEMA public TO spine_app;

-- Table DML permissions only
GRANT SELECT, INSERT, UPDATE, DELETE ON ALL TABLES IN SCHEMA public TO spine_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT SELECT, INSERT, UPDATE, DELETE ON TABLES TO spine_app;

-- Sequence permissions
GRANT USAGE, SELECT, UPDATE ON ALL SEQUENCES IN SCHEMA public TO spine_app;
ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT USAGE, SELECT, UPDATE ON SEQUENCES TO spine_app;
```

---

## 3. Network Binding Hardening

In `postgresql.conf` and `pg_hba.conf`:
1. `listen_addresses` is bound strictly to `localhost,127.0.0.1` and the specific host-only WSL virtual IP.
2. Wildcard CIDR `0.0.0.0/0` was removed from `pg_hba.conf`.
3. Client authentication uses SCRAM-SHA-256 password hashing.

---

## 4. Programmatic Verification Matrix

In `tests/postgres_least_privilege_test.rs`:
1. Queries `pg_roles` for `spine_app`:
   - `rolsuper`: **`false`**
   - `rolcreaterole`: **`false`**
   - `rolcreatedb`: **`false`**
   - `rolbypassrls`: **`false`**
2. Executes `CREATE ROLE test_unauth`: returns PostgreSQL error `42501` (Permission Denied).
3. Executes `ALTER SYSTEM`: returns PostgreSQL error `42501` (Must be superuser).
4. Executes standard application DML queries (`SELECT`, `INSERT`, `UPDATE`): succeeds normally.
