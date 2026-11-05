# Administrator Bootstrap, Initial Setup, and Disaster Recovery Guide

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Scope:** Initial Root Account Provisioning, CLI Administration, Emergency Credential Reset, and Disaster Recovery Procedures.

---

## 1. Initial System Administrator Bootstrap

On a clean Spine Forge installation, user self-registration is disabled by default (`SPINE_ALLOW_REGISTRATION=false`) to prevent unauthorized sign-ups on private instances.

The initial administrator account must be provisioned using the dedicated `spine-api bootstrap-admin` CLI command.

### 1.1. Execution Command
```bash
spine-api bootstrap-admin \
  --username admin \
  --email security@company.com \
  --database-url "postgres://spine_app:SecretDbPassword@postgres.internal.net:5432/spine_forge"
```

### 1.2. Cryptographic Behavior
1. **Uniqueness Enforcement:** Verifies that no account with the specified username or email already exists in PostgreSQL.
2. **CSPRNG Password Generation:** Generates a 32-character random alphanumeric password using `rand::thread_rng()`.
3. **Argon2id Hashing:** Hashes the password using Argon2id with recommended OWASP memory/time parameters.
4. **Audit Logging:** Inserts a structured audit event into `audit_events` with action `system.bootstrap_admin`.
5. **Output Display:** Outputs the temporary password once to stdout.

---

## 2. Post-Bootstrap Configuration Steps

1. **Log In to Forge UI or CLI:**
   - Navigate to `https://forge.example.com/login` (or `http://127.0.0.1:3000/login`).
   - Authenticate using username `admin` and the generated 32-character password.
2. **Rotate Password:**
   - Under Settings > Account > Security, change the temporary password to an enterprise password managed by your password manager or SSO.
3. **Provision Primary Organization:**
   - Create the default root organization (e.g. `core` or your company name).
4. **Register User SSH Keys & Personal Access Tokens (PATs):**
   - Upload operator SSH public keys (`id_ed25519.pub`) for Git-over-SSH push access.
   - Generate a PAT with `admin` and `repo` scopes for CI/CD automation.

---

## 3. Emergency Disaster Recovery: Lost Admin Credentials

If all administrator credentials or session keys are lost, an emergency administrator or password reset can be executed directly through the database layer or CLI.

### Option A: Database-Level Emergency Reset Token
Generate an emergency reset token hash and inject it directly into PostgreSQL:

```bash
# 1. Generate random token
TOKEN=$(openssl rand -hex 32)
TOKEN_HASH=$(echo -n "$TOKEN" | sha256sum | awk '{print $1}')
EXPIRY=$(date -d "+1 hour" +%s%N)

# 2. Insert into PostgreSQL
psql -U spine_app -d spine_forge -c "
  INSERT INTO password_reset_tokens (token_hash, user_id, expires_at_nanos)
  SELECT '$TOKEN_HASH', id, $EXPIRY FROM users WHERE username = 'admin';
"

# 3. Complete password reset via API:
curl -X POST https://forge.example.com/api/v1/auth/reset-password \
  -H "Content-Type: application/json" \
  -d "{\"token\": \"$TOKEN\", \"new_password\": \"NewSuperSecretAdminPass2026!\"}"
```

### Option B: Full Database Restoration
If database corruption occurs, restore the database from the latest encrypted backup archive:
```bash
spine-api backup restore \
  --input /backups/latest.spinebak \
  --target-storage /var/lib/spine/storage \
  --passphrase "${SPINE_BACKUP_PASSPHRASE}"

psql -U spine_app -d spine_forge < /var/lib/spine/storage/database/snapshot.sql
```
