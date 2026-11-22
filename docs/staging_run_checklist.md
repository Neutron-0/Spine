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
1. **Configure Authoritative Staging FQDN:**
   Define and export the single canonical `${STAGING_FQDN}` used across all TLS and ingress steps:
   ```bash
   export STAGING_FQDN="git-staging.example-corp.net" # Set to operator's real assigned public domain
   ```
   Validate that the domain is configured and is neither empty nor a placeholder:
   ```bash
   if [ -z "${STAGING_FQDN}" ] || [[ "${STAGING_FQDN}" =~ (example\.com|<.*>|placeholder|localhost|127\.0\.0\.1) ]]; then
     echo "FATAL: STAGING_FQDN must be configured with a real, authoritative public domain name (got '${STAGING_FQDN}'). Placeholders and localhost are forbidden." >&2
     exit 1
   fi
   ```
2. **Public DNS Resolution:** Configure an external DNS `A` or `CNAME` record mapping `${STAGING_FQDN}` strictly to the public edge IP of the staging host.
3. **Publicly Trusted CA Certificate & SAN Validation:** Obtain a certificate issued by a recognized public root CA (e.g. Let's Encrypt / ACME, AWS ACM, DigiCert). Confirm that the certificate Subject Alternative Name (SAN) explicitly includes `${STAGING_FQDN}` prior to edge deployment:
   ```bash
   CERT_SANS=$(openssl x509 -in "${SPINE_TLS_CERT_FILE}" -noout -text | grep -A1 "Subject Alternative Name" | tr -d '\n\r ')
   if ! echo "${CERT_SANS}" | grep -q "DNS:${STAGING_FQDN}"; then
     echo "FATAL: Certificate SAN (${CERT_SANS}) does not include the configured STAGING_FQDN (DNS:${STAGING_FQDN})" >&2
     exit 1
   fi
   ```
4. **Full Chain Assembly & Mount:** Ensure the certificate bundle includes the leaf certificate and all intermediate CA certificates (`fullchain.pem`), deployed with restrictive permissions (`chmod 600`):
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

2. **Configure Staging Environment & Unique Project Parameters:**
   ```bash
   # Assign and validate a unique, isolated staging Compose project name:
   export STAGING_PROJECT_NAME="spine-staging-$(openssl rand -hex 4)"
   if [[ ! "${STAGING_PROJECT_NAME}" =~ ^spine-staging-[0-9a-fA-F]{6,16}$ ]]; then
     echo "FATAL: STAGING_PROJECT_NAME '${STAGING_PROJECT_NAME}' is invalid! Must match ^spine-staging-[0-9a-fA-F]{6,16}$" >&2
     exit 1
   fi

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

To definitively prove that backup immutability is enforced by the **S3 Object Lock Compliance WORM engine** rather than unconfigured principals, expired credentials, an IAM deny, or a bucket policy deny, operators can use either the automated rehearsal script or the manual procedure below.

#### Automated Rehearsal Tool (Recommended)
Run [`scripts/rehearse_s3_compliance.py`](../scripts/rehearse_s3_compliance.py):
```bash
python scripts/rehearse_s3_compliance.py \
  --bucket "<staging-bucket-name>" \
  --target-key "backups/production-rehearsal/<backup-archive-filename>.spnebak" \
  --version-id "${ARCHIVE_VERSION_ID}" \
  --expected-role "SpineForge-AuditRole" \
  --profile spine-auditor \
  --region us-east-1
```
The automated script executes the full 5-phase verification:
1. Asserts caller identity matches `SpineForge-AuditRole` via `--profile spine-auditor` (rejects raw key CLI args).
2. Simulates IAM policy to verify `s3:DeleteObject` and `s3:DeleteObjectVersion` are explicitly `allowed`, and audits bucket policy for absence of unconditional denies.
3. Performs positive control deletion of an unlocked test object (must succeed to prove genuine deletion permissions).
4. Probes version-specific deletion of the locked archive (strictly asserts failure with `AccessDenied` / HTTP 403; fails as a critical breach if deletion succeeds).
5. Asserts `head-object` and `get-object-retention` confirm the exact version remains intact with `COMPLIANCE` mode and future retain date.

---

#### Manual CLI Verification Procedure
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

3. **Configure Auditor Role Profile & Prove Deletion Authorization (IAM & Bucket Policy):**
   > [!IMPORTANT]
   > A generic `AccessDenied` alone does NOT prove S3 Object Lock Compliance WORM enforcement: an unconfigured principal, expired credentials, an IAM deny, or a bucket policy deny would also yield `AccessDenied`.
   >
   > To definitively prove that refusal is caused by the **S3 Object Lock Compliance WORM engine**, the test procedure must:
   > 1. Use the audit role through a configured AWS CLI profile (`--profile spine-auditor`) rather than passing raw credentials inline.
   > 2. Verify the effective assumed identity is `SpineAuditVerificationRole`.
   > 3. Verify via IAM policy simulation that the role is **explicitly permitted** to perform `s3:DeleteObject` and `s3:DeleteObjectVersion` without explicit denies.
   > 4. Verify the S3 bucket policy contains no explicit denies against the auditor role.
   > 5. Require the version-specific delete to fail and confirm via `head-object` and `get-object-retention` that the exact same version remains with active `COMPLIANCE` retention metadata.

   ```bash
   # Retrieve AuditRoleArn from CloudFormation stack outputs:
   AUDIT_ROLE_ARN=$(aws --profile spine-staging cloudformation describe-stacks \
     --stack-name spine-staging-storage \
     --query "Stacks[0].Outputs[?OutputKey=='AuditRoleArn'].OutputValue" \
     --output text)

   # Configure dedicated staging auditor profile using AWS CLI role assumption (zero inline raw credentials):
   aws configure set profile.spine-auditor.role_arn "${AUDIT_ROLE_ARN}"
   aws configure set profile.spine-auditor.source_profile spine-staging
   aws configure set profile.spine-auditor.role_session_name "spine-auditor-worm-verification"

   # 1. Verify effective caller identity:
   CALLER_ARN=$(aws --profile spine-auditor sts get-caller-identity --query "Arn" --output text)
   if ! echo "${CALLER_ARN}" | grep -q "assumed-role/SpineForge-AuditRole-"; then
     echo "FATAL: Effective caller identity is not SpineAuditVerificationRole (${CALLER_ARN})" >&2
     exit 1
   fi

   # 2. Verify IAM policy permissions and check for explicit policy denies:
   SIMULATION_JSON=$(aws --profile spine-staging iam simulate-principal-policy \
     --policy-source-arn "${AUDIT_ROLE_ARN}" \
     --action-names "s3:DeleteObject" "s3:DeleteObjectVersion" \
     --resource-arns "arn:aws:s3:::<staging-bucket-name>/backups/production-rehearsal/*")

   DEL_OBJ_DECISION=$(echo "${SIMULATION_JSON}" | jq -r '.EvaluationResults[] | select(.EvalActionName=="s3:DeleteObject") | .EvalDecision')
   DEL_VER_DECISION=$(echo "${SIMULATION_JSON}" | jq -r '.EvaluationResults[] | select(.EvalActionName=="s3:DeleteObjectVersion") | .EvalDecision')

   if [ "${DEL_OBJ_DECISION}" != "allowed" ] || [ "${DEL_VER_DECISION}" != "allowed" ]; then
     echo "FATAL: IAM policy simulation failed! Expected 'allowed' without explicit denies, got DeleteObject=${DEL_OBJ_DECISION}, DeleteObjectVersion=${DEL_VER_DECISION}" >&2
     echo "Simulation details: ${SIMULATION_JSON}" >&2
     exit 1
   fi

   # 3. Verify S3 bucket policy does not contain explicit deny statements against the auditor role:
   BUCKET_POLICY=$(aws --profile spine-staging s3api get-bucket-policy --bucket "<staging-bucket-name>" --query "Policy" --output text 2>/dev/null || echo "{}")
   if echo "${BUCKET_POLICY}" | jq -e '.Statement[] | select(.Effect=="Deny" and (.Action | (if type=="array" then . else [.] end) | any(. == "s3:DeleteObject" or . == "s3:DeleteObjectVersion" or . == "s3:*")))' >/dev/null 2>&1; then
     AUDIT_ROLE_DENIED=$(echo "${BUCKET_POLICY}" | jq -r --arg role "${AUDIT_ROLE_ARN}" '
       .Statement[] | select(.Effect=="Deny" and (.Action | (if type=="array" then . else [.] end) | any(. == "s3:DeleteObject" or . == "s3:DeleteObjectVersion" or . == "s3:*")))
       | select(.Principal.AWS? == $role or (.Principal.AWS? | (if type=="array" then . else [.] end) | any(. == $role)))
     ')
     if [ -n "${AUDIT_ROLE_DENIED}" ]; then
       echo "FATAL: Bucket policy contains explicit Deny on deletion for auditor role (${AUDIT_ROLE_DENIED})! Cannot test S3 WORM." >&2
       exit 1
     fi
   fi
   ```

4. **Attempt Version-Specific Deletion with Configured Auditor Profile (Safe Under `set -e`):**
   ```bash
   # Retrieve exact VersionId of the uploaded backup archive:
   ARCHIVE_VERSION_ID=$(aws --profile spine-staging s3api list-object-versions \
     --bucket <staging-bucket-name> \
     --prefix backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --query "Versions[?IsLatest==\`true\`].VersionId" \
     --output text)

   if [ -z "${ARCHIVE_VERSION_ID}" ] || [ "${ARCHIVE_VERSION_ID}" = "None" ]; then
     echo "FATAL: Could not resolve latest VersionId for archive backups/production-rehearsal/<backup-archive-filename>.spnebak" >&2
     exit 1
   fi

   # Execute version-specific deletion using the configured profile under safe set -e handling:
   set +e
   DELETE_OUTPUT=$(aws --profile spine-auditor s3api delete-object \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --version-id "${ARCHIVE_VERSION_ID}" 2>&1)
   DELETE_EXIT_CODE=$?
   set -e

   # Evaluation 1: A successful delete is a critical test failure under Object Lock Compliance
   if [ "${DELETE_EXIT_CODE}" -eq 0 ]; then
     echo "FATAL: S3 version-specific delete succeeded! S3 Object Lock Compliance WORM failed to protect archive version ${ARCHIVE_VERSION_ID}." >&2
     exit 1
   fi

   # Evaluation 2: Capture and assert that rejection is AccessDenied / HTTP 403
   if echo "${DELETE_OUTPUT}" | grep -qE "(AccessDenied|403|Access Denied)"; then
     echo "CONFIRMED: S3 Object Lock Compliance WORM rejected version deletion with AccessDenied (HTTP 403)."
   else
     echo "FATAL: Deletion failed with unexpected non-WORM error (${DELETE_OUTPUT})." >&2
     echo "Unrelated errors or credential failures cannot satisfy the WORM Compliance verification gate." >&2
     exit 1
   fi
   ```

5. **Confirm Version Preservation & Compliance Metadata via `head-object` and `get-object-retention`:**
   Immediately following the rejected deletion, assert that the exact same version remains in place and retains valid Compliance retention metadata:
   ```bash
   # 1. Assert Compliance retention is active:
   RETENTION_JSON=$(aws --profile spine-staging s3api get-object-retention \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --version-id "${ARCHIVE_VERSION_ID}")

   echo "${RETENTION_JSON}" | grep -q '"Mode": "COMPLIANCE"' || {
     echo "FATAL: ObjectLockMode is not COMPLIANCE for version ${ARCHIVE_VERSION_ID}: ${RETENTION_JSON}" >&2
     exit 1
   }

   # 2. Assert head-object metadata confirms exact version and future retention date:
   HEAD_OBJECT_JSON=$(aws --profile spine-staging s3api head-object \
     --bucket <staging-bucket-name> \
     --key backups/production-rehearsal/<backup-archive-filename>.spnebak \
     --version-id "${ARCHIVE_VERSION_ID}")

   echo "${HEAD_OBJECT_JSON}" | grep -q '"ObjectLockMode": "COMPLIANCE"' || {
     echo "FATAL: ObjectLockMode is not COMPLIANCE in head-object metadata: ${HEAD_OBJECT_JSON}" >&2
     exit 1
   }

   echo "${HEAD_OBJECT_JSON}" | grep -q '"ObjectLockRetainUntilDate"' || {
     echo "FATAL: ObjectLockRetainUntilDate missing from object version ${ARCHIVE_VERSION_ID}" >&2
     exit 1
   }

   echo "${HEAD_OBJECT_JSON}" | grep -q "\"VersionId\": \"${ARCHIVE_VERSION_ID}\"" || {
     echo "FATAL: VersionId mismatch in head-object verification" >&2
     exit 1
   }
   ```
   **Pass Criteria for Object Lock Compliance WORM:**
   - Identity verified as `SpineAuditVerificationRole` with `s3:DeleteObject` and `s3:DeleteObjectVersion` evaluated as `allowed` in IAM and unblocked in bucket policy.
   - Deletion attempt via `--profile spine-auditor` fails with `AccessDenied` / HTTP 403.
   - `head-object` and `get-object-retention` confirm the exact version `${ARCHIVE_VERSION_ID}` remains stored with `ObjectLockMode=COMPLIANCE` and future retain date.

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

The canonical health readiness route is `/health/ready`, which is explicitly routed by `deploy/nginx/ingress.conf` (`location ~ ^/(health|healthz|readyz)`) to `http://spine-api:3000/health/ready` and matches the container healthcheck definition in `docker-compose.prod.yml`. All probes strictly target the configured `${STAGING_FQDN}` without insecure TLS bypass flags (`-k`/`--insecure`).

#### Automated Ingress Prober (Recommended)
Operators can execute both Probe 4a (HTTPS trust, SAN, HTTP 200, headers) and Probe 4b (plain HTTP port 80 redirect to HTTPS) with a single command via [`scripts/probe_staging_ingress.py`](../scripts/probe_staging_ingress.py) or [`scripts/probe_staging_ingress.sh`](../scripts/probe_staging_ingress.sh):

```bash
# Python prober:
python scripts/probe_staging_ingress.py --fqdn "${STAGING_FQDN}"

# Or POSIX bash wrapper:
./scripts/probe_staging_ingress.sh "${STAGING_FQDN}"
```

---

#### Manual Probe Procedures

#### Probe 4a: Public CA HTTPS Ingress Trust Probe (Port 443)
Execute the TLS trust probe from an external client outside the staging local network using the host's standard root CA store:
```bash
# 1. Issue GET request over HTTPS with redirects disabled (no -L), dumping headers to verify exact HTTP 200 and security headers:
# (Note: curl -fsS alone can succeed on a redirect; dumping headers allows strictly asserting HTTP 200)
HTTPS_HEADER_DUMP=$(curl -s -S -D - -o /dev/null "https://${STAGING_FQDN}/health/ready")

# 2. Extract final HTTP status code and assert strictly 200 (reject redirects or errors):
HTTPS_STATUS_LINE=$(echo "${HTTPS_HEADER_DUMP}" | grep -E "^HTTP/[12\.]+" | tail -n 1)
HTTPS_STATUS_CODE=$(echo "${HTTPS_STATUS_LINE}" | awk '{print $2}')

if [ "${HTTPS_STATUS_CODE}" != "200" ]; then
  echo "FATAL: HTTPS readiness probe did not return HTTP 200! Received status line: '${HTTPS_STATUS_LINE}'" >&2
  echo "Full response headers:" >&2
  echo "${HTTPS_HEADER_DUMP}" >&2
  exit 1
fi

# 3. Assert mandatory security headers:
echo "${HTTPS_HEADER_DUMP}" | grep -qi "^Strict-Transport-Security:.*max-age=" || {
  echo "FATAL: Missing or invalid Strict-Transport-Security header in HTTPS response" >&2
  exit 1
}
echo "${HTTPS_HEADER_DUMP}" | grep -qi "^X-Content-Type-Options: nosniff" || {
  echo "FATAL: Missing or invalid X-Content-Type-Options header in HTTPS response" >&2
  exit 1
}
echo "${HTTPS_HEADER_DUMP}" | grep -qi "^X-Frame-Options: SAMEORIGIN" || {
  echo "FATAL: Missing or invalid X-Frame-Options header in HTTPS response" >&2
  exit 1
}
echo "${HTTPS_HEADER_DUMP}" | grep -qi "^Referrer-Policy: strict-origin-when-cross-origin" || {
  echo "FATAL: Missing or invalid Referrer-Policy header in HTTPS response" >&2
  exit 1
}

# 4. Strictly validate TLS certificate chain and hostname matching using openssl s_client:
# -verify_return_error causes openssl to exit with an error code if the root CA chain fails verification
TLS_VERIFY_OUTPUT=$(echo | openssl s_client -connect "${STAGING_FQDN}:443" -servername "${STAGING_FQDN}" -verify_return_error 2>&1)
TLS_VERIFY_EXIT=$?

if [ ${TLS_VERIFY_EXIT} -ne 0 ]; then
  echo "FATAL: OpenSSL certificate chain verification failed for ${STAGING_FQDN}!" >&2
  echo "${TLS_VERIFY_OUTPUT}" >&2
  exit 1
fi

# 5. Assert certificate Subject Alternative Name explicitly matches ${STAGING_FQDN}:
LIVE_SAN=$(echo "${TLS_VERIFY_OUTPUT}" | openssl x509 -noout -text 2>/dev/null \
  | grep -A1 "Subject Alternative Name" \
  | grep -o "DNS:${STAGING_FQDN}")

if [ -z "${LIVE_SAN}" ]; then
  echo "FATAL: Live TLS certificate presented at ${STAGING_FQDN}:443 does not match SAN DNS:${STAGING_FQDN}" >&2
  exit 1
fi
```
> [!IMPORTANT]
> Never use `-k` or `--insecure`. The connection must succeed using the client OS standard root CA store.

**Pass Criteria:**
- HTTP status code is strictly `200 OK` (not a 3xx redirect or 4xx/5xx error).
- TLS handshake completes without certificate errors or warnings; root CA chain passes `-verify_return_error`.
- Server certificate SAN strictly matches `${STAGING_FQDN}`.
- Certificate issuer is a recognized public CA (e.g., Let's Encrypt, AWS ACM, DigiCert; not self-signed, not localhost).
- Modern TLS protocol negotiated (`TLSv1.3` or `TLSv1.2` with PFS cipher suites).
- Security headers verified:
  - `Strict-Transport-Security: max-age=63072000; includeSubDomains; preload`
  - `X-Content-Type-Options: nosniff`
  - `X-Frame-Options: SAMEORIGIN`
  - `Referrer-Policy: strict-origin-when-cross-origin`

#### Probe 4b: Plain HTTP to HTTPS Mandatory 301 Redirection Probe (Port 80)
Execute the HTTP redirection probe against port 80 using a **GET** request with redirects **disabled** (curl does not follow redirects without `-L`):
```bash
# Issue GET request with redirects disabled, capturing headers:
HTTP_HEADER_DUMP=$(curl -s -S -D - -o /dev/null "http://${STAGING_FQDN}/health/ready")

# 1. Assert HTTP 301 Moved Permanently status line:
if ! echo "${HTTP_HEADER_DUMP}" | grep -qE "^HTTP/[12\.]+ 301"; then
  echo "FATAL: Port 80 did not return HTTP 301! Full response headers:" >&2
  echo "${HTTP_HEADER_DUMP}" >&2
  exit 1
fi

# 2. Assert Location header points strictly to canonical HTTPS URL:
LOCATION_HEADER=$(echo "${HTTP_HEADER_DUMP}" | grep -i "^Location:" | tr -d '\r' | awk '{print $2}')
EXPECTED_LOCATION="https://${STAGING_FQDN}/health/ready"

if [ "${LOCATION_HEADER}" != "${EXPECTED_LOCATION}" ]; then
  echo "FATAL: Location header '${LOCATION_HEADER}' does not match expected '${EXPECTED_LOCATION}'" >&2
  exit 1
fi
```
**Pass Criteria:**
- HTTP status code is strictly `301 Moved Permanently`.
- `Location` header is strictly `https://${STAGING_FQDN}/health/ready` (preserving path and query string).
- No unencrypted payload or sensitive data returned over plain HTTP.

---

## 3. Post-Rehearsal Staging Cleanup Procedures

To ensure zero cost leakage, clean state boundaries, and secure credential handling after staging verification:

### A. Standby Application & Database Teardown (Safe Project Scoping)
Teardown must strictly target only the unique staging project name generated for this run, preventing accidental disruption of production or co-located stacks:
```bash
# 1. Validate STAGING_PROJECT_NAME syntax before teardown:
if [[ ! "${STAGING_PROJECT_NAME}" =~ ^spine-staging-[0-9a-fA-F]{6,16}$ ]]; then
  echo "FATAL: Refusing teardown! STAGING_PROJECT_NAME '${STAGING_PROJECT_NAME}' does not match expected staging pattern ^spine-staging-[0-9a-fA-F]{6,16}$" >&2
  exit 1
fi

# 2. Tear down only containers, networks, and volumes belonging to this specific staging run:
docker compose -p "${STAGING_PROJECT_NAME}" down -v --remove-orphans

# 3. Assert zero lingering containers or volumes for this project:
LINGERING_CONTAINERS=$(docker ps -a -q --filter "name=${STAGING_PROJECT_NAME}")
if [ -n "${LINGERING_CONTAINERS}" ]; then
  echo "FATAL: Lingering staging containers found after teardown: ${LINGERING_CONTAINERS}" >&2
  exit 1
fi

LINGERING_VOLUMES=$(docker volume ls -q --filter "name=${STAGING_PROJECT_NAME}_")
if [ -n "${LINGERING_VOLUMES}" ]; then
  echo "FATAL: Lingering staging volumes found after teardown: ${LINGERING_VOLUMES}" >&2
  exit 1
fi
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
| **Backup Scheduling & RPO Freshness Alerting** | Hourly automated backups, Alert webhook delivery on $>60$m age | **LOCALLY TESTED / PREPARED** | Automation installer (`scripts/install_backup_automation.sh`) and verifier (`scripts/verify_deployed_operations.py`) tested with 14 unit tests; pending target host deployment enrollment. |
| **AWS S3 Object Lock Compliance WORM** | S3 WORM Compliance Mode, $\ge 30$-day retention, versioned delete refusal | **BLOCKED / UNVERIFIED** | 5-phase rehearsal tool (`scripts/rehearse_s3_compliance.py`, 8 unit tests) and CloudFormation template ready; blocked on real AWS account & bucket. |
| **Public CA TLS Gateway & HTTPS Redirection** | Valid CA Trust Chain (no `-k`), Port 80 $\to$ 443 301 Redirect | **BLOCKED / UNVERIFIED** | Ingress verification probers (`scripts/probe_staging_ingress.py`, 11 unit tests) ready; blocked on public DNS record & public CA certificate. |

**Overall Production-Readiness Status:** **NOT PRODUCTION-READY** (Held pending real AWS Object Lock and public CA TLS staging verification).

---

## 5. Evidence Capture & Redaction Standards

When recording staging execution results:
- **Redaction Mandate:** Redact all tokens, passwords, passphrases, AWS account numbers, and private IPs (`[REDACTED_TOKEN]`, `[REDACTED_PASSPHRASE]`, `123456789012` $\to$ `[REDACTED_AWS_ACCOUNT]`).
- **Required Metadata:** For each command, record the exact command invocation, execution timestamp (UTC), exit status code, and sanitized output snippet.
