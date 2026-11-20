# Spine Forge — Out-of-Band Storage Backup & Disaster Recovery Runbook

**Document Identifier:** `SPINE-OPS-RUNBOOK-OOB-01`  
**Classification:** Operational Runbook / Disaster Recovery Standard Operating Procedure  
**Target Profile:** `core-forge-v1` (Strict Production Role Separation, TLS Ingress, Multi-Node Storage)  
**Applicability:** Production cluster volumes exceeding the 15 MiB (`MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES`) single-frame cluster RPC backup limit.

---

## 1. Executive Summary & Architectural Context

Spine Forge provides two distinct backup and disaster recovery mechanisms:

1. **In-Band Cluster RPC Backups (`SPNEBAK v2`):**
   - Coordinated directly by `spine-api` and `spine-coordinator` over single-frame cluster RPC calls (`ClusterRpcRequest` / `ClusterRpcResponse`).
   - Governed by `MAX_REMOTE_PARTICIPANT_PAYLOAD_BYTES` = `15 MiB` (`15,728,640` bytes) compressed payload per node.
   - Enforced by `BoundedVecWriter` which aborts capture immediately if a participant's compressed snapshot exceeds 15 MiB, preventing memory spikes and buffer exhaustion under 64 MiB JSON RPC framing.

2. **Out-of-Band (OOB) Storage Volume Snapshots (This Runbook):**
   - Required whenever repository data, chunk data, or database volumes on any individual storage node exceed 15 MiB compressed.
   - Leverages storage-level atomic point-in-time snapshotting (Cloud Block Storage EBS/PD, OpenZFS, or LVM) or quiesced streaming archives.
   - Leverages Spine's built-in **Writer Draining Barrier** and **Monotonic Fencing Generation Tokens** to guarantee zero in-flight mutations, cross-volume consistency, and fail-closed post-restore verification.

---

## 2. Production Storage Participant Inventory

A valid Spine Forge production deployment consists of 7 canonical stateful resources across 4 host containers:

| Volume Identifier | Container / Service | Host / Mount Path | Authoritative Contents |
|---|---|---|---|
| `postgres_data` | `spine-forge-postgres` | `/var/lib/postgresql/data` | Relational metadata, user accounts, sessions, permissions, audit logs. |
| `coordinator_data` | `spine-coordinator` | `/var/lib/spine` | Raft consensus log, cluster topology, `.coordinator_fencing_generation`. |
| `storage_1_data` | `spine-storage-node-1` | `/var/lib/spine/node-1` | Repositories (`repos/`), BLAKE3 chunks (`chunks/`), manifests (`manifests/`), `.fencing_generation`. |
| `storage_2_data` | `spine-storage-node-2` | `/var/lib/spine/node-2` | Repositories (`repos/`), BLAKE3 chunks (`chunks/`), manifests (`manifests/`), `.fencing_generation`. |
| `storage_3_data` | `spine-storage-node-3` | `/var/lib/spine/node-3` | Repositories (`repos/`), BLAKE3 chunks (`chunks/`), manifests (`manifests/`), `.fencing_generation`. |
| `spine_storage` | `spine-forge-api` | `/var/lib/spine/storage` | API runtime caches, scratch files, Git upload-pack temp buffers. |
| `spine_ssh` | `spine-forge-api` | `/var/lib/spine/ssh` | Authoritative host Ed25519 host key pair (`host_key_ed25519`). |

### 2.1 Supported Production Storage Topology & Topology Enforcement

Under deployment profile `core-forge-v1`, **Docker Compose with local volume mounts (`driver: local`) is the sole verified and supported production target**.

> [!IMPORTANT]
> **Strict Storage Driver Constraints:**
> - Non-local volume drivers (e.g. NFS, CIFS/SMB, GlusterFS, or unverified distributed network volumes) lack the strict POSIX `fsync` flush guarantees and atomic directory rename semantics required by:
>   1. **`spine-coordinator` Raft Consensus:** Requires durable fsync write-ahead logs to avoid state divergence.
>   2. **PostgreSQL Relational Engine:** Shared storage without strict POSIX file locks risks silent database corruption.
>   3. **BLAKE3 Content-Addressed Chunk Store:** Atomic commit of immutable chunks depends on atomic same-filesystem renames.
> - Network filesystems lacking node-level fencing introduce high risk of split-brain writes during failover.
> - Both `scripts/backup_oob.sh` and `scripts/restore_oob.sh` enforce this via `verify_supported_storage_topology()`. If any volume is configured with a driver other than `local`, the scripts immediately abort fail-closed prior to acquiring the maintenance barrier or touching cluster volumes.
> - Multi-node Kubernetes/CSI or cloud storage workflows requiring distributed fencing are outside the verified production scope of profile `core-forge-v1`. Any deployment attempting non-local drivers will be rejected.

---

## 3. Pre-Flight Sizing & Decision Matrix

Before initiating a backup, operators should inspect the compressed payload size of the storage nodes:

```bash
# Measure compressed size of storage node 1
docker exec spine-storage-node-1 sh -c \
  "tar -czf - -C /var/lib/spine/node-1 repos chunks manifests 2>/dev/null | wc -c"
```

- **If size $\le 15,728,640$ bytes (15 MiB):** In-band backup (`spine-api` automated backup) may be utilized.
- **If size $> 15,728,640$ bytes:** Execute the Out-of-Band procedure detailed below.

---

## 4. Consistent Point-in-Time Out-of-Band Backup Procedure

### Phase 1: Drain In-Flight Mutations, Enter Maintenance Mode & Verify Participant Unanimity

To prevent tearing across distributed chunk stores and PostgreSQL, in-flight writes must be drained, cluster maintenance mode acquired, and all participants audited for generation unanimity before any snapshots are taken.

1. **Acquire Maintenance Barrier via API / CLI:**
   ```bash
   # Issue maintenance barrier command to spine-api (or via spine admin CLI)
   curl -s -X POST http://127.0.0.1:3000/api/v1/admin/maintenance/acquire \
     -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" \
     -H "Content-Type: application/json" \
     -d '{"reason": "Scheduled Out-of-Band Snapshot", "timeout_seconds": 300}'
   ```

2. **Verify Zero Active Writers:**
   Check readiness endpoint diagnostics to confirm writer count has reached 0:
   ```bash
   curl -s http://127.0.0.1:3000/health/diagnostics | jq .
   # Expected output:
   # {
   #   "in_maintenance": true,
   #   "active_writers": 0,
   #   "current_fencing_generation": 42
   # }
   ```

3. **Direct Pre-Capture Audit Across All Distributed Participants:**
   Directly query the coordinator and all storage nodes inside their volumes to verify generation unanimity before snapshotting:
   ```bash
   API_GEN=$(curl -s http://127.0.0.1:3000/health/diagnostics | grep -o '"current_fencing_generation":[0-9]*' | cut -d: -f2)
   COORD_GEN=$(docker compose -f docker-compose.prod.yml exec -T coordinator cat /var/lib/spine/.coordinator_fencing_generation | tr -d '[:space:]')
   NODE_1_GEN=$(docker compose -f docker-compose.prod.yml exec -T storage-node-1 cat /var/lib/spine/node-1/.fencing_generation | tr -d '[:space:]')
   NODE_2_GEN=$(docker compose -f docker-compose.prod.yml exec -T storage-node-2 cat /var/lib/spine/node-2/.fencing_generation | tr -d '[:space:]')
   NODE_3_GEN=$(docker compose -f docker-compose.prod.yml exec -T storage-node-3 cat /var/lib/spine/node-3/.fencing_generation | tr -d '[:space:]')

   if [ "${COORD_GEN}" != "${API_GEN}" ] || [ "${NODE_1_GEN}" != "${API_GEN}" ] || \
      [ "${NODE_2_GEN}" != "${API_GEN}" ] || [ "${NODE_3_GEN}" != "${API_GEN}" ]; then
     echo "FATAL: Pre-capture audit detected generation mismatch across participants. Aborting backup."
     curl -s -X POST http://127.0.0.1:3000/api/v1/admin/maintenance/release \
       -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}"
     exit 1
   fi
   GENERATION_TOKEN="${API_GEN}"
   echo "Unanimous generation token: ${GENERATION_TOKEN}"
   ```

### Phase 2: Capture Multi-Volume Backup (Canonical Production Procedure)

With active mutations drained and write permits rejected with HTTP 503, capture all volumes consistently.

> [!IMPORTANT]
> **Canonical Certified Production Procedure (Quiesced Multi-Volume Archive Streaming):**
> This procedure is the authoritative, cloud-agnostic disaster recovery standard for Spine Forge clusters when storage exceeds 15 MiB. It operates uniformly across Docker Compose, Kubernetes, and bare-metal environments, and is backed by the turnkey script `scripts/backup_oob.sh` and automated regression test `test_out_of_band_storage_backup_and_restore_above_15mib`.
> *(For optional cloud hypervisor volume snapshots such as AWS EBS / GCP PD, see Appendix A).*

> [!SECURITY]
> **Mandatory Authenticated AEAD Encryption (SPNEBAK v2) & Credential Isolation:**
> Under production deployment profile `core-forge-v1`, unencrypted plaintext backups containing PostgreSQL relational tables, SSH host private keys, and repository data are **strictly forbidden**.
> - All backups must be authenticated and encrypted using the platform-supported **SPNEBAK v2** format (`AES-256-GCM`, 100,000 PBKDF2 iterations, per-chunk monotonic nonces, and authenticated manifest).
> - `SPINE_BACKUP_PASSPHRASE_FILE` (or `SPINE_BACKUP_PASSPHRASE`) is mandatory.
> - **Credential Protection from Process Arguments & Container Environment:**
>   - The backup encryption passphrase is **never passed in process arguments (`argv`) or container environment variables (`-e`)**.
>   - Passphrase files are mounted read-only into `/run/secrets/backup_passphrase:ro` inside the container, and CLI commands consume `--passphrase-file /run/secrets/backup_passphrase`.
>   - If an operator provides `SPINE_BACKUP_PASSPHRASE` via the host shell, the script writes it to an ephemeral `0600` secret file, unsets the shell environment variable immediately, mounts the secret file, and securely shreds/removes the temporary file on exit.
> - **Comprehensive Cleanup Trap (`EXIT ERR INT TERM`):** All intermediate plaintext staging directories (`.staging`) and unencrypted bundles (`.plaintext_bundle.tar.gz`) are guaranteed to be purged on every exit path — including snapshot failure, encryption error, verification abort, or operator interruption — while retaining the encrypted `backup.spnebak` archive and reporting any cleanup failures.
> - Restrictive permissions (`0700` directory, `0600` encrypted archive) are enforced.

Execute the automated backup script:
```bash
# Set environment variables and execute canonical backup
export SPINE_API_URL="http://127.0.0.1:3000"
export SPINE_ADMIN_TOKEN="<administrative-token>"
export BACKUP_ROOT="/mnt/backups/spine"
export SPINE_BACKUP_PASSPHRASE_FILE="/etc/spine/backup_passphrase"

# Off-host S3-compatible immutable replication (ENFORCED by default in production)
export SPINE_BACKUP_REQUIRE_REMOTE=1  # 1 = Enforced (fails if unset or failed), 0 = Allow intentionally local-only
export SPINE_BACKUP_REMOTE_S3_URI="s3://spine-disaster-recovery-vault/cluster-backups"
export SPINE_BACKUP_OBJECT_LOCK_MODE="GOVERNANCE" # or COMPLIANCE
export SPINE_BACKUP_RETENTION_DAYS="30"
# export SPINE_S3_ENDPOINT_URL="https://s3.us-east-1.amazonaws.com"

./scripts/backup_oob.sh
```

### Phase 2c: Off-Host Replication (S3 Object Lock) & Policy Enforcement

1. **Mandatory Off-Site Replication Policy (`SPINE_BACKUP_REQUIRE_REMOTE=1`):**
   Under production deployment profile `core-forge-v1`, off-site replication is **enforced by default** (`SPINE_BACKUP_REQUIRE_REMOTE=1`).
   - If `SPINE_BACKUP_REMOTE_S3_URI` is not configured or the `aws` CLI is missing, `scripts/backup_oob.sh` **fails closed immediately at pre-flight** (prior to acquiring the maintenance barrier or taking any snapshots).
   - Local-only backups can only be created by explicitly overriding the policy (`SPINE_BACKUP_REQUIRE_REMOTE=0`), which emits an operational warning acknowledging the disaster recovery limitation.

2. **Off-Host Replication & Cryptographic Byte-Level Verification:**
   When `SPINE_BACKUP_REMOTE_S3_URI` is configured, `scripts/backup_oob.sh` replicates the encrypted `backup.spnebak` and `SHA256SUMS` manifest to the designated object store:
   - Configures S3 WORM / Object Lock immutability (`--object-lock-mode` and `--object-lock-retain-until-date`) to guarantee tamper-proof and ransomware-resistant retention.
   - **Cryptographic SHA-256 Digest Gate:** Rather than relying solely on file sizes, the script streams the uploaded archive from the remote object store (`aws s3 cp ... - | sha256sum`) and cryptographically compares the remote SHA-256 digest against the local archive before reporting backup success. Any mismatch or corruption immediately fails the backup with exit status 1.

3. **Explicit Disaster Recovery Deployment Scope Limitation:**
   > [!WARNING]
   > **Deployment Limitation for Local-Only Backups:**
   > If `SPINE_BACKUP_REQUIRE_REMOTE=0` is explicitly set to permit local-only backups, archives remain exclusively on host storage under `BACKUP_ROOT`. Local-only backups **do not provide full disaster-recovery readiness** against total host hardware failure, disk controller destruction, or datacenter loss. Production installations must configure an off-host object store with Object Lock retention enabled.

Or execute the canonical snapshot steps manually:
```bash
umask 077
BACKUP_DIR="/mnt/backups/spine/$(date +%Y%m%d_%H%M%S)_gen_${GENERATION_TOKEN}"
STAGING_DIR="${BACKUP_DIR}/.staging"
mkdir -p "${STAGING_DIR}"
chmod 700 "${BACKUP_DIR}" "${STAGING_DIR}"

# 0. Record target fencing generation token
echo "${GENERATION_TOKEN}" > "${STAGING_DIR}/FENCING_GENERATION"

# 1. Database snapshot via compose
docker compose -f docker-compose.prod.yml exec -T postgres pg_dumpall -U spine_admin | gzip > "${STAGING_DIR}/postgres_dump.sql.gz"

# 2. Coordinator state
docker compose -f docker-compose.prod.yml exec -T coordinator tar -czf - -C /var/lib/spine . > "${STAGING_DIR}/coordinator.tar.gz"

# 3. Storage node volumes
for NODE in 1 2 3; do
  docker compose -f docker-compose.prod.yml exec -T "storage-node-${NODE}" tar -czf - -C "/var/lib/spine/node-${NODE}" . > "${STAGING_DIR}/storage_node_${NODE}.tar.gz"
done

# 4. Host SSH keys & API storage
docker compose -f docker-compose.prod.yml exec -T spine-api tar -czf - -C /var/lib/spine/ssh . > "${STAGING_DIR}/spine_ssh.tar.gz"
docker compose -f docker-compose.prod.yml exec -T spine-api tar -czf - -C /var/lib/spine/storage . > "${STAGING_DIR}/spine_storage.tar.gz"

# 5. Checksum manifest of captured participants
(cd "${STAGING_DIR}" && sha256sum * > SHA256SUMS)

# 6. Authenticated AEAD Encryption (SPNEBAK v2)
tar -czf "${BACKUP_DIR}/.plaintext_bundle.tar.gz" -C "${STAGING_DIR}" .
spine-api backup encrypt --input "${BACKUP_DIR}/.plaintext_bundle.tar.gz" --output "${BACKUP_DIR}/backup.spnebak"

# 7. Securely purge unencrypted plaintext staging data
rm -rf "${BACKUP_DIR}/.plaintext_bundle.tar.gz" "${STAGING_DIR}"
(cd "${BACKUP_DIR}" && sha256sum backup.spnebak > SHA256SUMS)
chmod 600 "${BACKUP_DIR}"/*
chmod 700 "${BACKUP_DIR}"
```

### Phase 3: Release Maintenance Mode & Resume Service

```bash
curl -s -X POST http://127.0.0.1:3000/api/v1/admin/maintenance/release \
  -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}"
```

Verify the cluster returns to full health:
```bash
curl -s -f http://127.0.0.1:3000/health/ready || echo "Cluster not ready!"
```

---

## 5. Point-in-Time Out-of-Band Disaster Recovery Procedure

Follow these steps to restore a Spine Forge cluster from an out-of-band snapshot archive (or execute `./scripts/restore_oob.sh <path-or-s3-uri>`).

```bash
# Example 1: Local restore
export SPINE_BACKUP_PASSPHRASE_FILE="/etc/spine/backup_passphrase"
./scripts/restore_oob.sh /mnt/backups/spine/20261120_120000Z_gen_42/backup.spnebak

# Example 2: Remote off-host S3 restore
export SPINE_BACKUP_PASSPHRASE_FILE="/etc/spine/backup_passphrase"
./scripts/restore_oob.sh s3://spine-disaster-recovery-vault/cluster-backups/20261120_120000Z_gen_42/backup.spnebak
```

### Step 0: Authenticate, Decrypt & Validate Archive (SPNEBAK v2 AEAD Envelope)

All out-of-band backups are stored encrypted using Spine's authenticated backup envelope (`SPNEBAK v2` AES-256-GCM AEAD, 100,000 PBKDF2 iterations). Before stopping services or modifying storage volumes, the incoming archive must be validated and authenticated fail-closed.

1. **Remote Download & Comprehensive Cleanup (if S3 URI provided):**
   When an `s3://` target is specified, `scripts/restore_oob.sh` creates an isolated staging directory under `/tmp` (mode `0700`), downloads `backup.spnebak` and `SHA256SUMS`, verifies the checksum, and stages the archive.
   - **Guaranteed Cleanup Trap (`EXIT`, `ERR`, `INT`, `TERM`):** The remote staging directory is strictly purged on **every exit path** — upon successful restore completion, during download failure, upon checksum mismatch, or on signal interruption (`SIGINT`/`SIGTERM`).
   - Live live volume rollback archives remain safely preserved in `/var/tmp/.pre_restore_rollback_*` if failure occurs after volumes were modified.

2. **Verify Envelope Magic & Integrity (Fail-Closed Gate):**
   ```bash
   # Check envelope header magic
   MAGIC=$(head -c 8 "${BACKUP_DIR}/backup.spnebak" 2>/dev/null || true)
   if [ "${MAGIC}" != "SPNEBAK"$'\x02' ]; then
     echo "FATAL: Archive is not an authenticated SPNEBAK v2 backup. Aborting restore."
     exit 1
   fi

   # Verify backup checksum
   (cd "${BACKUP_DIR}" && sha256sum -c SHA256SUMS)
   ```

3. **Authenticate & Decrypt with Isolated Credential Mounting:**
   ```bash
   DECRYPT_DIR="${BACKUP_DIR}/.decrypted_restore_$(date +%s)"
   mkdir -m 0700 -p "${DECRYPT_DIR}"

   # Decrypt using host binary or container fallback with mounted secret file
   if command -v spine-api >/dev/null 2>&1; then
     spine-api backup decrypt --passphrase-file /etc/spine/backup_passphrase \
       --input "${BACKUP_DIR}/backup.spnebak" --output - | tar -xzf - -C "${DECRYPT_DIR}"
   else
     # Note: Secret file is mounted read-only; no secrets in container environment or CLI args
     docker compose -f docker-compose.prod.yml run --rm -i --no-deps \
       -v /etc/spine/backup_passphrase:/run/secrets/backup_passphrase:ro \
       spine-api \
       backup decrypt --passphrase-file /run/secrets/backup_passphrase \
       --input - --output - < "${BACKUP_DIR}/backup.spnebak" | tar -xzf - -C "${DECRYPT_DIR}"
   fi

   # Verify inner payload SHA256SUMS
   (cd "${DECRYPT_DIR}" && sha256sum -c SHA256SUMS)
   ```
   > [!IMPORTANT]
   > **Immediate Fail-Closed Cleanup Trap:** `scripts/restore_oob.sh` installs the cleanup trap (`EXIT ERR INT TERM`) immediately upon creating `${DECRYPT_DIR}`. Any failure during AEAD authentication, decryption, archive extraction, or inner checksum verification (`sha256sum -c SHA256SUMS`) immediately triggers the trap and purges `${DECRYPT_DIR}` from disk, guaranteeing zero plaintext database dumps or SSH host private keys remain exposed.

### Step 1: Quiesce Traffic, Stop Containers & Verify Full Quiescence

1. Ingress and API shutdown to prevent clients from attempting writes during recovery:
   ```bash
   docker compose -f docker-compose.prod.yml stop ingress spine-api
   ```
2. Stop all storage, consensus, and database components:
   ```bash
   docker compose -f docker-compose.prod.yml stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend
   ```
3. **Verify All Containers Stopped (Fail-Closed Gate):**
   ```bash
   RUNNING=$(docker compose -f docker-compose.prod.yml ps --services --filter "status=running")
   if [ -n "${RUNNING}" ]; then
     echo "FATAL: Services still running: ${RUNNING}. Aborting restore to prevent data corruption."
     exit 1
   fi
   ```

### Step 2: Mandatory Pre-Restore Archiving & Clean Target Volume Extraction

To prevent old deleted files from persisting after restore, target volumes are not overlaid. Existing files across all volumes are preserved in an external rollback directory (`${BACKUP_DIR}/.pre_restore_rollback_<timestamp>`) outside the volume mount paths.

> [!SECURITY]
> **Encrypted Rollback Artifacts (Zero Plaintext Persistence):**
> Under production security standards, pre-restore rollback archives are **never persisted in plaintext**. All 7 participant volumes are streamed directly through `encrypt_stream` into authenticated `SPNEBAK v2` AEAD envelopes (`*.spnebak`) using the same backup passphrase, with strict `0600` file permissions and `0700` directory permissions. This ensures sensitive live data — including PostgreSQL relational tables and SSH host private keys — remains protected for the entire recovery window.

> [!IMPORTANT]
> **Zero Modifications Gate:** All 7 participant rollback archives (`coordinator`, `storage_node_1/2/3`, `spine_ssh`, `spine_storage`, `postgres_data`) MUST be generated without error suppression and verified via streaming AEAD MAC authentication and tar integrity checks (`decrypt_stream < file | tar -tzf -`) BEFORE any volume directory is emptied or modified. If any rollback archive creation or verification fails, restore aborts immediately leaving all live volumes completely untouched.

> [!CAUTION]
> **Automatic Fail-Closed Rollback Handler (`scripts/restore_oob.sh`):**
> If an unrecoverable failure occurs during volume extraction, post-restore verification, or service startup:
> 1. `scripts/restore_oob.sh` executes `restore_cleanup()` automatically via bash trap (`EXIT ERR INT TERM`).
> 2. **Mandatory Quiescence Verification Before Rollback Extraction:** The handler executes `docker compose stop` without error suppression and verifies that all services are stopped (`RUNNING=0`). If any container is running or refuses to stop, rollback extraction is strictly aborted (`ROLLBACK_FAILED=1`), leaving all volumes untouched and cluster stopped to prevent data tearing.
> 3. Rollback extraction commands stream on the fly from encrypted `*.spnebak` archives into container volume extractors without writing intermediate plaintext files to disk.
> 4. Following extraction, every volume is checked for critical marker files (`.coordinator_fencing_generation`, `.fencing_generation`, `PG_VERSION`).
> 5. If any rollback step fails or fewer than all 7 participants are verified, `restore_oob.sh` logs a `CRITICAL FATAL` alert identifying the failure, **strictly leaves all cluster services stopped**, and exits with code 2. It will NEVER report rollback success unless all 7 volumes are confirmed restored and verified.
> 6. Staged rollback archives in `${ROLLBACK_DIR}` are retained intact in encrypted `SPNEBAK v2` format for operator emergency triage.

1. **Phase 2a: Mandatory Pre-Restore Archiving & Integrity Verification (Volumes Untouched):**
   ```bash
   ROLLBACK_DIR="${BACKUP_DIR}/.pre_restore_rollback_$(date +%s)"
   mkdir -p "${ROLLBACK_DIR}"
   chmod 700 "${ROLLBACK_DIR}"

   # Archive and encrypt all 7 participants on the fly (SPNEBAK v2)
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar coordinator \
     -czf - -C /var/lib/spine . | encrypt_stream > "${ROLLBACK_DIR}/coordinator.spnebak"

   for NODE in 1 2 3; do
     docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar "storage-node-${NODE}" \
       -czf - -C "/var/lib/spine/node-${NODE}" . | encrypt_stream > "${ROLLBACK_DIR}/storage_node_${NODE}.spnebak"
   done

   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar spine-api \
     -czf - -C /var/lib/spine/ssh . | encrypt_stream > "${ROLLBACK_DIR}/spine_ssh.spnebak"
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar spine-api \
     -czf - -C /var/lib/spine/storage . | encrypt_stream > "${ROLLBACK_DIR}/spine_storage.spnebak"
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar postgres \
     -czf - -C /var/lib/postgresql/data . | encrypt_stream > "${ROLLBACK_DIR}/postgres_data.spnebak"

   chmod 600 "${ROLLBACK_DIR}"/*

   # Verify all 7 rollback archives exist, are non-empty, and pass AEAD MAC & tar integrity checks
   for ARCHIVE in coordinator storage_node_1 storage_node_2 storage_node_3 spine_ssh spine_storage postgres_data; do
     FILE="${ROLLBACK_DIR}/${ARCHIVE}.spnebak"
     if [ ! -s "${FILE}" ] || ! decrypt_stream < "${FILE}" | tar -tzf - >/dev/null 2>&1; then
       echo "FATAL: Pre-restore rollback archive ${FILE} is missing or corrupted. Aborting restore with zero volumes modified."
       exit 1
     fi
   done
   echo "All 7 pre-restore rollback archives verified valid and encrypted. Safe to proceed with volume modification."
   ```

2. **Phase 2b: Clean Target Volumes & Extract Incoming Backup:**
   ```bash
   # Clean ephemeral staging in storage nodes
   for NODE in 1 2 3; do
     docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
       -c "rm -rf /var/lib/spine/node-${NODE}/.staged_restore_* /var/lib/spine/node-${NODE}/.old_restore_* 2>/dev/null || true"
   done

   # Empty target volumes and extract incoming backup cleanly from authenticated decrypted payload
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh coordinator \
     -c "rm -rf /var/lib/spine/* /var/lib/spine/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine" < "${DECRYPT_DIR}/coordinator.tar.gz"

   for NODE in 1 2 3; do
     docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh "storage-node-${NODE}" \
       -c "rm -rf /var/lib/spine/node-${NODE}/* /var/lib/spine/node-${NODE}/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/node-${NODE}" < "${DECRYPT_DIR}/storage_node_${NODE}.tar.gz"
   done

   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh spine-api \
     -c "rm -rf /var/lib/spine/ssh/* /var/lib/spine/ssh/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/ssh" < "${DECRYPT_DIR}/spine_ssh.tar.gz"
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh spine-api \
     -c "rm -rf /var/lib/spine/storage/* /var/lib/spine/storage/.[!.]* 2>/dev/null || true; tar -xzf - -C /var/lib/spine/storage" < "${DECRYPT_DIR}/spine_storage.tar.gz"
   ```

### Step 3: Automated Fail-Closed Fencing Generation Verification

Run the automated verification script to confirm that all nodes match the target generation token directly inside their respective volumes. **Execution will abort immediately with exit code 1 if any node diverges.**

```bash
TARGET_GEN=$(cat "${DECRYPT_DIR}/FENCING_GENERATION" | tr -d '[:space:]')
FAILED=0

COORD_GEN=$(docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint cat coordinator \
  /var/lib/spine/.coordinator_fencing_generation 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
if [ "${COORD_GEN}" != "${TARGET_GEN}" ]; then
  echo "FATAL: Coordinator generation (${COORD_GEN}) does not match target generation (${TARGET_GEN})."
  FAILED=1
else
  echo "Coordinator generation: ${COORD_GEN} [OK]"
fi

for NODE in 1 2 3; do
  NODE_GEN=$(docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint cat "storage-node-${NODE}" \
    "/var/lib/spine/node-${NODE}/.fencing_generation" 2>/dev/null | tr -d '[:space:]' || echo "MISSING")
  if [ "${NODE_GEN}" != "${TARGET_GEN}" ]; then
    echo "FATAL: Storage node ${NODE} generation (${NODE_GEN}) diverged from target generation (${TARGET_GEN})."
    FAILED=1
  else
    echo "Storage node ${NODE} generation: ${NODE_GEN} [OK]"
  fi
done

if [ ${FAILED} -ne 0 ]; then
  echo "CRITICAL: Generation divergence detected across restored participants."
  echo "RESTORE HALTED. DO NOT MANUALLY OVERWRITE OR EDIT .fencing_generation FILES."
  echo "Proceed to the Defined Fencing Recovery Procedure below."
  exit 1
fi
echo "All nodes verified at fencing generation ${TARGET_GEN}. Safe to proceed."
```

> [!CAUTION]
> **STRICT FAIL-CLOSED POLICY — DO NOT MANUALLY OVERWRITE FENCING GENERATIONS:**
> All storage nodes and the coordinator **MUST** report the identical integer generation matching the recorded `${GENERATION_TOKEN}` from Phase 1.
> 
> **NEVER MANUALLY EDIT, OVERWRITE, OR FORCE A NODE'S `.fencing_generation` OR `.coordinator_fencing_generation` FILE.** Overriding fencing tokens manually bypasses distributed consensus safeguards, allows stale nodes to accept conflicting writes, and masks split-brain volume tearing.
> 
> **Defined Fencing Recovery Procedure (If Divergence Occurs):**
> 1. **Halt the Restore Immediately:** Do not proceed to Step 4 or start the cluster services.
> 2. **Audit Volume Provenance:** Inspect the snapshot metadata and creation timestamps of all restored volumes. Confirm that all volumes originated from the exact same atomic snapshot group (`SpineFencingGen=${GENERATION_TOKEN}`).
> 3. **Re-restore Out-of-Sync Participant:** If one node was accidentally restored from a different snapshot point, wipe that node's volume mount and re-restore strictly from the correct snapshot group.
> 4. **Quorum Reconciliation:** If the coordinator generation matches the snapshot token and at least a quorum (2 of 3) of storage nodes match, start only the matching nodes and run the coordinator anti-entropy repair command:
>    ```bash
>    docker compose -f docker-compose.prod.yml run --rm coordinator \
>      repair-generation --expected-gen "${GENERATION_TOKEN}"
>    # (Note: flag format is also accepted: --repair-generation --expected-gen "${GENERATION_TOKEN}")
>    ```
> 5. **Unresolvable Divergence:** If generation consensus cannot be established, abort the restore and fall back to the previous known consistent snapshot set.

### Step 4: Storage Node Integrity Check

Run an offline integrity check on each storage node volume to verify chunk manifests and repository references:
```bash
for NODE in 1 2 3; do
  docker compose -f docker-compose.prod.yml run --rm "storage-node-${NODE}" \
    /usr/local/bin/spine-storage-node --verify-integrity --data-dir "/var/lib/spine/node-${NODE}"
done
```

### Step 5: Clean PostgreSQL Restoration & Verification

1. **Preserve Existing PostgreSQL Data to External Rollback Dir & Clear PGDATA:**
   ```bash
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint tar postgres \
     -czf - -C /var/lib/postgresql/data . > "${ROLLBACK_DIR}/postgres_data.tar.gz"
   docker compose -f docker-compose.prod.yml run --rm --no-deps --entrypoint sh postgres \
     -c "rm -rf /var/lib/postgresql/data/* /var/lib/postgresql/data/.[!.]* 2>/dev/null || true"
   ```
2. **Start Fresh PostgreSQL Container & Wait for Readiness:**
   ```bash
   docker compose -f docker-compose.prod.yml up -d postgres
   until docker compose -f docker-compose.prod.yml exec -T postgres pg_isready -U spine_admin >/dev/null 2>&1; do
     sleep 1
   done
   ```
3. **Drop Empty Database Created by Container Init:**
   ```bash
   docker compose -f docker-compose.prod.yml exec -T postgres psql -U spine_admin -d postgres -v ON_ERROR_STOP=1 -c "
     SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname = 'spine_forge' AND pid <> pg_backend_pid();
     DROP DATABASE IF EXISTS spine_forge;
   "
   ```
4. **Restore Database SQL Dump (with ON_ERROR_STOP=1 & Idempotent Role Adaptation):**
   ```bash
   gzip -dc "${DECRYPT_DIR}/postgres_dump.sql.gz" | \
     sed -E 's/^CREATE ROLE ([^;]+);/DO \$\$ BEGIN CREATE ROLE \1; EXCEPTION WHEN duplicate_object THEN NULL; END \$\$;/' | \
     docker compose -f docker-compose.prod.yml exec -T postgres \
     psql -v ON_ERROR_STOP=1 -U spine_admin -d postgres
   ```
5. **Verify Restored Schema and Tables:**
   ```bash
   TABLE_COUNT=$(docker compose -f docker-compose.prod.yml exec -T postgres \
     psql -U spine_admin -d spine_forge -t -A -c \
     "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public';" 2>/dev/null || echo "0")
   if [ "${TABLE_COUNT}" -eq 0 ]; then
     echo "FATAL: Database restore produced 0 tables. Aborting."
     exit 1
   fi
   ```
6. **Start Core Cluster Infrastructure:**
   ```bash
   docker compose -f docker-compose.prod.yml up -d coordinator storage-node-1 storage-node-2 storage-node-3
   ```
7. **Wait for Health Checks to Pass:**
   ```bash
   docker compose -f docker-compose.prod.yml ps
   # Verify that postgres, coordinator, storage-node-1, storage-node-2, storage-node-3 are (healthy)
   ```
8. **Start Application Control Plane & Ingress:**
   ```bash
   docker compose -f docker-compose.prod.yml up -d spine-api spine-frontend ingress
   ```

### Step 6: Readiness Gate & Rollback Purge Gate

Execute the fail-closed readiness healthcheck:
```bash
curl -i http://127.0.0.1:3000/health/ready
```
**Required Response:**
- HTTP Status Code: `200 OK`

After `/health/ready` returns 200 OK, purge preserved rollback artifacts:
```bash
rm -rf "${ROLLBACK_DIR}"
```
- Body:
  ```json
  {
    "status": "ready",
    "database": "connected",
    "storage_root": "available",
    "coordinator": "connected",
    "storage_nodes_configured": 3,
    "storage_nodes_healthy": 3,
    "active_restore_journal": false
  }
  ```

Execute a verification Git clone and commit to confirm full end-to-end operation:
```bash
git clone https://forge.example.com/test-org/verification-repo.git /tmp/verify-repo
cd /tmp/verify-repo
git commit --allow-empty -m "OOB restore post-flight verification $(date)"
git push origin main
```

### Preserved Rollback Artifacts: Operator Inspection & Safe Removal

In the event of a failed restore that triggers automatic rollback, the pre-restore rollback directory (`${BACKUP_DIR}/.pre_restore_rollback_<timestamp>`) is intentionally retained on disk to permit forensic investigation without risk of data exposure.

1. **Format and Protection:**
   - All files within the rollback directory (`coordinator.spnebak`, `storage_node_1.spnebak`, `storage_node_2.spnebak`, `storage_node_3.spnebak`, `spine_ssh.spnebak`, `spine_storage.spnebak`, `postgres_data.spnebak`) are authenticated `SPNEBAK v2` AES-256-GCM AEAD envelopes.
   - Files are restricted to mode `0600` (owner read/write only) and the directory to `0700`.
   - No unencrypted plaintext is persisted on disk.

2. **Operator Inspection of Rollback Artifacts:**
   An operator can safely inspect the contents of any preserved participant archive using the CLI decryptor:
   ```bash
   export SPINE_BACKUP_PASSPHRASE="<backup-passphrase>"
   # Inspect PostgreSQL dump or file structure
   spine-api backup decrypt --input "${ROLLBACK_DIR}/postgres_data.spnebak" --output - | tar -tzf -

   # Inspect storage node or coordinator metadata
   spine-api backup decrypt --input "${ROLLBACK_DIR}/coordinator.spnebak" --output - | tar -tzf -
   ```
   Or via container fallback if the host binary is not installed:
   ```bash
   docker compose -f docker-compose.prod.yml run --rm -i --no-deps \
     -e SPINE_BACKUP_PASSPHRASE="${SPINE_BACKUP_PASSPHRASE}" spine-api \
     backup decrypt --input - --output - < "${ROLLBACK_DIR}/postgres_data.spnebak" | tar -tzf -
   ```

3. **Safe Removal After Investigation:**
   Once cluster health is verified or the incident review is completed, securely remove the preserved rollback directory:
   ```bash
   rm -rf "${ROLLBACK_DIR}"
   ```

---

## 6. Audit Trail & Compliance Verification

Every Out-of-Band backup and restore operation must be recorded in the site operations log with:
1. Snapshot timestamp (UTC)
2. Fencing generation token
3. Storage node volume SHA-256 digests
4. Verification test output
5. Operator signature

---

## Appendix A: Infrastructure-Level Volume Snapshotting (Optional Cloud / SAN Offload)

> [!NOTE]
> **Qualification Requirement for Infrastructure Snapshots:**
> Cloud block storage (AWS EBS, GCP Persistent Disk) and enterprise filesystems (OpenZFS, NetApp) can optionally be used by platform teams to snapshot volumes at the hypervisor level during Phase 1 maintenance mode. Because these mechanisms depend on cloud provider control planes and virtualization hypervisors outside the software control plane, platform engineering teams MUST qualify the multi-volume snapshot consistency of their infrastructure before deploying it in production:
> 1. Execute a pre-production dry run using `scripts/backup_oob.sh` as the baseline.
> 2. Create the cloud volume snapshot group while the cluster is in Phase 1 maintenance mode (`active_writers == 0`).
> 3. Restore the cloud snapshot to a test cluster and execute `scripts/restore_oob.sh` or Step 3 automated verification.
> 4. Ensure `spine-storage-node --verify-integrity` passes 100% on all restored volumes before approving the infrastructure snapshot mechanism.

### AWS EBS Multi-Volume Crash-Consistent Snapshot Group
```bash
aws ec2 create-snapshots \
  --instance-specification InstanceId="${INSTANCE_ID}" \
  --description "spine-forge-oob-backup-gen-${GENERATION_TOKEN}-$(date +%Y%m%d%H%M%S)" \
  --tag-specifications "ResourceType=snapshot,Tags=[{Key=SpineFencingGen,Value=${GENERATION_TOKEN}}]"
```

### OpenZFS Recursive Dataset Snapshot
```bash
SNAPSHOT_NAME="oob-backup-gen-${GENERATION_TOKEN}-$(date +%s)"
zfs snapshot -r zpool-spine/volumes@${SNAPSHOT_NAME}
```
