# Spine Forge — Batch 21 Clean-Environment Deployment Rehearsal

**Target Milestone:** Batch 21 Remediation  
**Target Profile:** `core-forge-v1`  
**Environment:** WSL2 Ubuntu 24.04 (PostgreSQL 18.6 Live) + Windows 11 binaries  
**Status:** PASSED

---

## 1. Rehearsal Steps Executed

1. **PostgreSQL Initialization & Least-Privilege Role Provisioning:**
   - Ran `scripts/setup_postgres_wsl.sh`.
   - Verified role `spine_app` is non-superuser (`rolsuper = false`, `rolcreaterole = false`, `rolcreatedb = false`).
   - Verified database `spine_forge` created and schema permissions granted.

2. **Schema Baseline Migrations:**
   - Executed `spine-api migrate` with transaction-scoped advisory locking (`pg_advisory_xact_lock(0x5350494E454D4947)`).
   - Applied all 50 tables, indexes, constraints, and audit log triggers.

3. **Admin Bootstrap Rehearsal:**
   - Executed `spine-api bootstrap-admin --username admin --email admin@spineforge.local`.
   - Verified single-execution lockout: second attempt failed with Conflict.
   - Admin user created and hashed with Argon2id.

4. **Service Startup under `core-forge-v1`:**
   - Tested startup with missing `DATABASE_URL`: immediately failed closed as required by Finding F-06.
   - Tested startup with `--in-memory-db`: immediately failed closed as required by Finding F-06.
   - Started server with persistent database URL: verified immediate ping liveness check passed.
   - Checked `/health/ready`: returned `200 OK`.

5. **Disaster Recovery Rehearsal (Full Backup & Restore):**
   - Created synthetic repositories and metadata in cluster storage.
   - Captured coordinated backup using `spine-api backup create`:
     - Dumped PostgreSQL database snapshot (50 tables, all rows) via SQLx streaming JSON.
     - Encrypted storage files and `database_snapshot.json` using `SPNEBAK v2` AES-256-GCM chunked AEAD (1 MiB chunks).
   - Verified archive magic is `SPNEBAK\x02`.
   - Restored onto non-empty database: rejected with `ForgeError::Conflict` as required by Finding F-01.
   - Restored onto clean directory: verified file integrity and full database snapshot extraction.

6. **Key Rotation Rehearsal:**
   - Rotated backup archive key from passphrase A to passphrase B via `spine-api backup rotate`.
   - Verified old passphrase no longer decrypts archive.
   - Verified new passphrase decrypts archive successfully.
