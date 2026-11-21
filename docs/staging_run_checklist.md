# Staging Verification Run Checklist: AWS Object Lock & Public CA TLS

> [!IMPORTANT]
> **Production Gate Status: BLOCKED**
> Production readiness sign-off is held until both external staging gates are verified against live AWS cloud infrastructure and public PKI root trust chains. No mocks, localhost rehearsals, or self-signed certificates satisfy these gates.

---

## 1. Prerequisite Infrastructure Provisioning

### A. AWS Staging Identity & Storage (CloudFormation Template)
Deploy the reviewed, parameterized CloudFormation template located at [`deploy/aws/staging-object-lock-storage.yaml`](../deploy/aws/staging-object-lock-storage.yaml):

```bash
aws --profile spine-staging cloudformation deploy \
  --template-file deploy/aws/staging-object-lock-storage.yaml \
  --stack-name spine-forge-staging-storage \
  --parameter-overrides \
      EnvironmentName=staging \
      BucketPrefix=spine-forge-backups \
      RetentionDays=30 \
      ObjectLockMode=COMPLIANCE \
  --capabilities CAPABILITY_NAMED_IAM
```

#### Required Input Parameters:
- `EnvironmentName`: `staging` (or `production`)
- `BucketPrefix`: Unique alphanumeric prefix (e.g. `spine-forge-backups`)
- `RetentionDays`: `30` (minimum WORM compliance duration)
- `ObjectLockMode`: `COMPLIANCE` (immutable, un-bypassable by any identity)
- `TrustedPrincipalArn`: ARN of the staging EC2/EKS execution role or user identity running the backup tool.

> [!CAUTION]
> Never print, hardcode, or paste raw AWS access keys or session tokens in chat, terminal history, commit logs, or reports. Use a local named AWS profile (`AWS_PROFILE=spine-staging`) or IAM instance/OIDC role.

---

### B. Public DNS & Trusted CA TLS Certificate
1. **Public DNS Record:** Configure an external DNS `A` or `CNAME` record (e.g., `git.spine-staging.example.com`) resolving to the public edge IP of the staging environment.
2. **Publicly Trusted CA Certificate:** Obtain a certificate issued by a recognized public CA (e.g., Let's Encrypt / ACME, AWS ACM, or DigiCert).
3. **Full Chain Assembly:** Ensure the certificate bundle includes the leaf certificate and all intermediate CA certificates (`fullchain.pem`).
4. **Proxy Mount:** Deploy the certificate and private key with restrictive permissions (`chmod 600`):
   - Ingress certificate: `SPINE_TLS_CERT_FILE=/etc/ssl/certs/fullchain.pem`
   - Ingress key: `SPINE_TLS_KEY_FILE=/etc/ssl/private/privkey.pem`

---

## 2. Staging Execution Procedures

### Step 1: Backup Execution & Off-Site S3 Replication
Export staging environment parameters:
```bash
export AWS_PROFILE=spine-staging
export SPINE_DEPLOYMENT_PROFILE=core-forge-v1
export SPINE_BACKUP_REQUIRE_REMOTE=1
export SPINE_BACKUP_OBJECT_LOCK_MODE=COMPLIANCE
export SPINE_BACKUP_RETENTION_DAYS=30
export SPINE_BACKUP_REMOTE_S3_URI="s3://<staging-bucket-name>/backups/production-rehearsal/"
export SPINE_BACKUP_PASSPHRASE_FILE="/path/to/secure/staging_passphrase.txt"
export SPINE_ADMIN_TOKEN_FILE="/path/to/secure/staging_admin_token.txt"
```

Execute out-of-band backup:
```bash
bash scripts/backup_oob.sh
```

- Confirm exit code is `0`.
- Verify stdout logs:
  - `status=acquired, fencing_generation=<G>` (owner token redacted)
  - Pre-capture storage audit across all participants matches `<G>`.
  - Authenticated encryption succeeded.
  - S3 upload completed and remote SHA-256 matched local hash.
  - Barrier released cleanly: `fencing_generation=<G>`.

---

### Step 2: S3 Object Lock & Cryptographic Verification
1. **Query Remote Object Retention Mode:**
   ```bash
   aws --profile spine-staging s3api get-object-retention \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak
   ```
   - Verify JSON output returns:
     - `"Mode": "COMPLIANCE"`
     - `"RetainUntilDate"` is $\ge 30$ days into the future.

2. **Download Archive and Verify Cryptographic Integrity:**
   ```bash
   aws --profile spine-staging s3 cp \
     s3://<staging-bucket-name>/backups/production-rehearsal/<backup-archive-filename>.spnebak \
     ./staging_verify.spnebak
   sha256sum ./staging_verify.spnebak
   ```
   - Verify computed SHA-256 matches manifest `SHA256SUMS`.

3. **Attempt Mutation/Deletion to Confirm Fail-Closed WORM Protection:**
   ```bash
   aws --profile spine-staging s3api delete-object \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak
   ```
   - Confirm deletion is rejected by AWS with `AccessDenied: Object is protected by Compliance retention`.

---

### Step 3: Standby Environment Restoration
1. Prepare an isolated standby Docker Compose stack with empty volumes.
2. Execute restore from the verified S3 artifact:
   ```bash
   export SPINE_RESTORE_REMOTE_S3_URI="s3://<staging-bucket-name>/backups/production-rehearsal/<backup-archive-filename>.spnebak"
   export SPINE_BACKUP_PASSPHRASE_FILE="/path/to/secure/staging_passphrase.txt"
   bash scripts/restore_oob.sh
   ```
3. Confirm exit code is `0`.
4. Verify standby environment consistency:
   - PostgreSQL schema and data tables populated.
   - Storage node generations match restored coordinator fencing generation.
   - SSH service accepts restored identities.
   - API diagnostics endpoint reports `/health/diagnostics` healthy and unpartitioned.

---

### Step 4: External Public CA HTTPS Ingress Verification
Execute probe from an external host outside the staging local network:
```bash
curl -Iv https://<staging-hostname>/health/ready
```
> [!IMPORTANT]
> Never use `-k` or `--insecure`. The connection must succeed using the client OS standard root CA store.

Verify handshake evidence:
- HTTP status `200 OK`.
- SSL certificate subject matches `<staging-hostname>`.
- Certificate issuer is a recognized public CA (not self-signed, not localhost).
- Modern TLS protocol negotiated (TLSv1.3 or TLSv1.2).
- `Strict-Transport-Security` header present and valid.

---

## 3. Evidence Capture & Redaction Standards

When recording staging execution results:
- **Redaction Mandate:** Redact all tokens, passwords, passphrases, AWS account numbers, and private IPs (`[REDACTED_TOKEN]`, `[REDACTED_PASSPHRASE]`, `123456789012` $\to$ `[REDACTED_AWS_ACCOUNT]`).
- **Required Metadata:** For each command, record the exact command invocation, execution timestamp (UTC), exit status code, and sanitized output snippet.
