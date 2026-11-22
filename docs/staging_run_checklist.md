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

### Step 1: Least-Privilege Role Assumption & Backup Execution
1. **Assume Least-Privilege SpineBackupRole:**
   ```bash
   # Retrieve BackupRoleArn from CloudFormation stack outputs:
   BACKUP_ROLE_ARN=$(aws --profile spine-staging cloudformation describe-stacks \
     --stack-name spine-staging-storage \
     --query "Stacks[0].Outputs[?OutputKey=='BackupRoleArn'].OutputValue" \
     --output text)

   # Assume role to obtain temporary credentials:
   BACKUP_SESSION=$(aws --profile spine-staging sts assume-role \
     --role-arn "${BACKUP_ROLE_ARN}" \
     --role-session-name "spine-backup-agent-session" \
     --duration-seconds 3600 \
     --query "Credentials.[AccessKeyId,SecretAccessKey,SessionToken]" \
     --output text)

   # Export temporary credentials (never print or log secrets):
   export AWS_ACCESS_KEY_ID=$(echo "${BACKUP_SESSION}" | awk '{print $1}')
   export AWS_SECRET_ACCESS_KEY=$(echo "${BACKUP_SESSION}" | awk '{print $2}')
   export AWS_SESSION_TOKEN=$(echo "${BACKUP_SESSION}" | awk '{print $3}')
   ```

2. **Configure Staging Environment Parameters:**
   ```bash
   export SPINE_DEPLOYMENT_PROFILE=core-forge-v1
   export SPINE_BACKUP_REQUIRE_REMOTE=1
   export SPINE_BACKUP_OBJECT_LOCK_MODE=COMPLIANCE
   export SPINE_BACKUP_RETENTION_DAYS=30
   export SPINE_BACKUP_REMOTE_S3_URI="s3://<staging-bucket-name>/backups/production-rehearsal/"
   export SPINE_BACKUP_PASSPHRASE_FILE="/path/to/secure/staging_passphrase.txt"
   export SPINE_ADMIN_TOKEN_FILE="/path/to/secure/staging_admin_token.txt"
   ```

3. **Execute Out-of-Band Backup:**
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
1. **Query Remote Object Retention Mode (using SpineBackupRole or Auditor Role):**
   ```bash
   aws s3api get-object-retention \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak
   ```
   - Verify JSON output returns:
     - `"Mode": "COMPLIANCE"`
     - `"RetainUntilDate"` is $\ge 30$ days into the future.

2. **Download Archive and Verify Cryptographic Integrity:**
   ```bash
   aws s3 cp \
     s3://<staging-bucket-name>/backups/production-rehearsal/<backup-archive-filename>.spnebak \
     ./staging_verify.spnebak
   sha256sum ./staging_verify.spnebak
   ```
   - Verify computed SHA-256 matches manifest `SHA256SUMS`.

3. **Attempt Version-Specific Deletion with Staging Auditor Identity:**
   > [!IMPORTANT]
   > Testing deletion using `SpineBackupRole` only proves IAM Deny, not S3 Object Lock Compliance retention. To verify S3 WORM enforcement, assume `SpineAuditVerificationRole`, which **explicitly allows** `s3:DeleteObject` and `s3:DeleteObjectVersion` in its IAM policy (see [`deploy/aws/staging-object-lock-storage.yaml`](../deploy/aws/staging-object-lock-storage.yaml)) and has no retention bypass permissions.
   >
   > Furthermore, in versioning-enabled S3 buckets, an unversioned `delete-object` merely places a delete marker. To test Object Lock Compliance WORM enforcement, the test MUST issue a **version-specific delete** (`--version-id`).

   ```bash
   # Retrieve AuditRoleArn from CloudFormation stack outputs:
   AUDIT_ROLE_ARN=$(aws --profile spine-staging cloudformation describe-stacks \
     --stack-name spine-staging-storage \
     --query "Stacks[0].Outputs[?OutputKey=='AuditRoleArn'].OutputValue" \
     --output text)

   # Assume auditor role:
   AUDIT_SESSION=$(aws --profile spine-staging sts assume-role \
     --role-arn "${AUDIT_ROLE_ARN}" \
     --role-session-name "spine-auditor-worm-verification" \
     --duration-seconds 900 \
     --query "Credentials.[AccessKeyId,SecretAccessKey,SessionToken]" \
     --output text)

   AUDIT_ACCESS_KEY=$(echo "${AUDIT_SESSION}" | awk '{print $1}')
   AUDIT_SECRET_KEY=$(echo "${AUDIT_SESSION}" | awk '{print $2}')
   AUDIT_TOKEN=$(echo "${AUDIT_SESSION}" | awk '{print $3}')

   # Retrieve exact VersionId of the uploaded backup archive:
   ARCHIVE_VERSION_ID=$(aws --profile spine-staging s3api list-object-versions \
     --bucket <staging-bucket-name> \
     --prefix backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --query "Versions[?IsLatest==\`true\`].VersionId" \
     --output text)

   # Attempt version-specific deletion with auditor credentials:
   AWS_ACCESS_KEY_ID="${AUDIT_ACCESS_KEY}" \
   AWS_SECRET_ACCESS_KEY="${AUDIT_SECRET_KEY}" \
   AWS_SESSION_TOKEN="${AUDIT_TOKEN}" \
   aws s3api delete-object \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --version-id "${ARCHIVE_VERSION_ID}"
   ```

   **Pass Criteria for Deletion Rejection:**
   - The command must exit with a non-zero exit code (`$? != 0`).
   - Standard error output must indicate `AccessDenied` / HTTP status code `403` (e.g. `An error occurred (AccessDenied) when calling the DeleteObject operation: Access Denied`).
   - Do NOT require an undocumented internal error message. Because `SpineAuditVerificationRole` is explicitly granted `s3:DeleteObject` and `s3:DeleteObjectVersion` in IAM, this refusal definitively proves that the rejection is enforced by the **AWS S3 Object Lock Compliance WORM engine**.

4. **Confirm Version Preservation & Compliance Metadata:**
   Immediately following the rejected deletion, assert that the version still exists and retains its Compliance metadata:
   ```bash
   aws --profile spine-staging s3api head-object \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --version-id "${ARCHIVE_VERSION_ID}"
   ```
   **Pass Criteria for Version Preservation:**
   - Command exits `0`.
   - Output contains:
     - `"ObjectLockMode": "COMPLIANCE"`
     - `"ObjectLockRetainUntilDate"` populated and $\ge 30$ days into the future.
     - `"VersionId"` matching `"${ARCHIVE_VERSION_ID}"`.
     - `"ContentLength"` matching the local backup archive size.

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

### Step 4: External Ingress Verification (HTTPS Trust & HTTP Redirection)

The canonical health readiness route is `/health/ready`, which is explicitly routed by `deploy/nginx/ingress.conf` (`location ~ ^/(health|healthz|readyz)`) to `http://spine-api:3000/health/ready` and matches the container healthcheck definition in `docker-compose.prod.yml`.

#### Probe 4a: Public CA HTTPS Ingress Trust Probe (Port 443)
Execute the TLS trust probe from an external client outside the staging local network using the host's standard root CA store:
```bash
curl -Iv https://<staging-hostname>/health/ready
```
> [!IMPORTANT]
> Never use `-k` or `--insecure`. The connection must succeed using the client OS standard root CA store.

**Pass Criteria:**
- HTTP status code `200 OK`.
- TLS handshake completes without certificate errors or warnings.
- Server certificate subject (CN or SAN) matches `<staging-hostname>`.
- Certificate issuer is a recognized public CA (e.g., Let's Encrypt, AWS ACM, DigiCert; not self-signed, not localhost).
- Modern TLS protocol negotiated (`TLSv1.3` or `TLSv1.2` with PFS cipher suites).
- Security headers verified:
  - `Strict-Transport-Security: max-age=63072000; includeSubDomains; preload`
  - `X-Content-Type-Options: nosniff`
  - `X-Frame-Options: SAMEORIGIN`

#### Probe 4b: Plain HTTP to HTTPS Mandatory 301 Redirection Probe (Port 80)
Execute the HTTP redirection probe against port 80:
```bash
curl -Iv http://<staging-hostname>/health/ready
```
**Pass Criteria:**
- HTTP status code `301 Moved Permanently`.
- `Location` header is strictly `https://<staging-hostname>/health/ready` (preserving path and query string).
- No unencrypted payload or sensitive data returned over plain HTTP.

---

## 3. Post-Rehearsal Staging Cleanup Procedures

To ensure zero cost leakage, clean state boundaries, and secure credential handling after staging verification:

### A. Standby Application & Database Teardown
```bash
# If running isolated restore Compose project:
docker compose -p spine-staging-restore down -v --remove-orphans

# If running primary staging Compose project:
docker compose -p spine-staging down -v --remove-orphans
```
Confirm all staging containers, networks, and named volumes are removed:
```bash
docker ps -a --filter "name=spine-staging"
docker volume ls --filter "name=spine-staging"
```

### B. Ephemeral Credential & Token Purge
Securely wipe local temporary credentials, session tokens, and passphrases:
```bash
# Securely overwrite and delete temporary token and passphrase files:
shred -u /path/to/secure/staging_admin_token.txt 2>/dev/null || rm -f /path/to/secure/staging_admin_token.txt
shred -u /path/to/secure/staging_passphrase.txt 2>/dev/null || rm -f /path/to/secure/staging_passphrase.txt
rm -f ./staging_verify.spnebak

# Unset all session credentials from current environment:
unset AWS_ACCESS_KEY_ID AWS_SECRET_ACCESS_KEY AWS_SESSION_TOKEN
unset BACKUP_SESSION AUDIT_SESSION
```

### C. AWS Infrastructure & S3 Object Lock Lifecycle
> [!CAUTION]
> **S3 Object Lock Compliance WORM Immutability:** Buckets with active Object Lock Compliance retention **cannot** be deleted or emptied by any AWS identity—including the AWS root account—until the retention period expires (`RetentionDays`).
>
> For temporary staging validation:
> - Keep `RetentionDays=1` (or minimum required) if the staging bucket is meant to be deleted post-verification.
> - For long-term staging environments, retain the CloudFormation stack (`spine-forge-staging-storage`) as dedicated staging infrastructure for future release candidate verification passes.

---

## 4. Current Status of Production-Readiness Gates

| Gate / Workstream | Target Standard | Current Status | Operational Evidence & Scope |
| :--- | :--- | :--- | :--- |
| **Release Artifact Signing & Anti-Tamper CI** | Ed25519 Detached Signatures, Independent Public Key Trust Anchor, Provenance Binding | **VERIFIED & CLOSED** | 12/12 anti-tamper tests passing in CI; trust anchor pinned at `deploy/security/spine-release-signing.pub`. |
| **PostgreSQL Database CI & Leaked Database Guard** | PostgreSQL 16 Service Container, Least-Privilege Roles, 0 Leaked Databases | **VERIFIED & CLOSED** | GitHub Actions run `45041563838` (commit `9eeb828`) 100% green; 13/13 integration & cluster recovery tests pass. |
| **Cluster Recovery Test Process Isolation** | Tracked PIDs, Zero Leaked Test Processes | **VERIFIED & CLOSED** | `TestChildGuard` in test harness; zero lingering coordinator/storage-node processes verified in CI. |
| **Production Compose Security & Resource Limits** | Non-root UID 10001, Read-only Filesystems, Enforced CPU/Memory Limits | **VERIFIED & CLOSED** | Verified in `exercise_production_compose.py` across all 8 production containers. |
| **Disaster Recovery RTO (1 GB Active Dataset)** | Nominal $<60$s Target | **MISSED (Nominal Target)** | Nominal target failed. Proved cold restore in **150.12s** & **155.02s** (mean **152.57s**, $\sigma = 3.46$s) on Windows/WSL2 NVMe host. |
| **Disaster Recovery RTO (Proposed SLA)** | $\le 5$ Minutes ($300$s) | **PROPOSED (Pending Approval)** | Local rehearsal measurements documented; pending formal service-owner sign-off. |
| **Disaster Recovery RTO (100 GB Scale)** | $< 15$ Minutes ($900$s) | **UNVERIFIED / PROJECTED** | Extrapolated; unverified pending large-scale staging measurement. |
| **Backup Scheduling & RPO Freshness Alerting** | Hourly automated backups, Alert webhook delivery on $>60$m age | **LOCALLY TESTED** | Systemd timers, cron definitions, and JSON webhook alert delivery loop verified locally; pending target host deployment. |
| **AWS S3 Object Lock Compliance WORM** | S3 WORM Compliance Mode, $\ge 30$-day retention, versioned delete refusal | **BLOCKED / UNVERIFIED** | CloudFormation template and runbook prepared; blocked on real AWS account & bucket. |
| **Public CA TLS Gateway & HTTPS Redirection** | Valid CA Trust Chain (no `-k`), Port 80 $\to$ 443 301 Redirect | **BLOCKED / UNVERIFIED** | Ingress configured; blocked on public DNS record & public CA certificate. |

**Overall Production-Readiness Status:** **NOT PRODUCTION-READY** (Held pending real AWS Object Lock and public CA TLS staging verification).

---

## 5. Evidence Capture & Redaction Standards

When recording staging execution results:
- **Redaction Mandate:** Redact all tokens, passwords, passphrases, AWS account numbers, and private IPs (`[REDACTED_TOKEN]`, `[REDACTED_PASSPHRASE]`, `123456789012` $\to$ `[REDACTED_AWS_ACCOUNT]`).
- **Required Metadata:** For each command, record the exact command invocation, execution timestamp (UTC), exit status code, and sanitized output snippet.
