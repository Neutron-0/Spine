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

### Phase 1: Drain In-Flight Mutations & Enter Maintenance Mode

To prevent tearing across distributed chunk stores and PostgreSQL, in-flight writes must be drained and cluster maintenance mode acquired.

1. **Acquire Maintenance Barrier via API / CLI:**
   ```bash
   # Issue maintenance barrier command to spine-api (or via spine admin CLI)
   curl -s -X POST http://127.0.0.1:3000/api/v1/admin/maintenance/acquire \
     -H "Authorization: Bearer ${SPINE_ADMIN_TOKEN}" \
     -H "Content-Type: application/json" \
     -d '{"reason": "Scheduled Out-of-Band Snapshot", "timeout_seconds": 300}'
   ```

2. **Verify Zero Active Writers & Monotonic Fencing Advance:**
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
   Record the current fencing generation (`GENERATION_TOKEN=42`).

### Phase 2: Capture Storage Snapshots

With active mutations drained and write permits rejected with HTTP 503, snapshot all volumes consistently.

#### Strategy A: Cloud Block Storage (AWS EBS / GCP Persistent Disk)
Create a multi-volume crash-consistent snapshot group containing all underlying block volumes:
```bash
# Example AWS CLI volume snapshot group
aws ec2 create-snapshots \
  --instance-specification InstanceId=i-0123456789abcdef0 \
  --description "spine-forge-oob-backup-gen-${GENERATION_TOKEN}-$(date +%Y%m%d%H%M%S)" \
  --tag-specifications "ResourceType=snapshot,Tags=[{Key=SpineFencingGen,Value=${GENERATION_TOKEN}}]"
```

#### Strategy B: Enterprise Filesystem Snapshots (OpenZFS)
```bash
SNAPSHOT_NAME="oob-backup-gen-${GENERATION_TOKEN}-$(date +%s)"
zfs snapshot -r zpool-spine/volumes@${SNAPSHOT_NAME}
```

#### Strategy C: Quiesced Archive Streaming (Host Filesystem / Object Storage)
If snapshot-capable block storage is not present, stream compressed tarballs directly to an external backup destination:
```bash
BACKUP_DIR="/mnt/backups/spine/$(date +%Y%m%d_%H%M%S)_gen_${GENERATION_TOKEN}"
mkdir -p "${BACKUP_DIR}"

# 1. Database snapshot
docker exec spine-forge-postgres pg_dumpall -U spine_admin | gzip > "${BACKUP_DIR}/postgres_dump.sql.gz"

# 2. Coordinator state
docker exec spine-coordinator tar -czf - -C /var/lib/spine . > "${BACKUP_DIR}/coordinator.tar.gz"

# 3. Storage node volumes
for NODE in 1 2 3; do
  docker exec spine-storage-node-${NODE} tar -czf - -C /var/lib/spine/node-${NODE} . > "${BACKUP_DIR}/storage_node_${NODE}.tar.gz"
done

# 4. Host SSH keys & API storage
docker exec spine-forge-api tar -czf - -C /var/lib/spine/ssh . > "${BACKUP_DIR}/spine_ssh.tar.gz"
docker exec spine-forge-api tar -czf - -C /var/lib/spine/storage . > "${BACKUP_DIR}/spine_storage.tar.gz"

# 5. Checksum manifest
cd "${BACKUP_DIR}" && sha256sum * > SHA256SUMS
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

Follow these steps to restore a Spine Forge cluster from an out-of-band snapshot.

### Step 1: Quiesce Traffic & Stop Running Containers

1. Ingress shutdown to prevent clients from attempting writes during recovery:
   ```bash
   docker compose -f docker-compose.prod.yml stop ingress spine-api
   ```
2. Stop all storage and consensus components:
   ```bash
   docker compose -f docker-compose.prod.yml stop storage-node-1 storage-node-2 storage-node-3 coordinator postgres spine-frontend
   ```

### Step 2: Restore Volumes & Clean Leftover Staging Files

1. **Restore Volumes:** Attach/restore the block volume snapshots or untar the backup archive into the corresponding volume directories.
2. **Remove Ephemeral Staging Directories:**
   Ensure no leftover partial staging directories exist:
   ```bash
   for NODE in 1 2 3; do
     rm -rf /var/lib/spine/node-${NODE}/.staged_restore_*
     rm -rf /var/lib/spine/node-${NODE}/.old_restore_*
   done
   ```

### Step 3: Fencing Generation Verification

Verify that `.fencing_generation` on all storage nodes and `.coordinator_fencing_generation` on the coordinator match the target recovery generation token:
```bash
for NODE in 1 2 3; do
  echo -n "Node ${NODE} generation: "
  cat /var/lib/spine/node-${NODE}/.fencing_generation
  echo ""
done
echo -n "Coordinator generation: "
cat /var/lib/spine/.coordinator_fencing_generation
echo ""
```
All nodes **MUST** report the identical integer generation. If any node has diverged, set the generation file to the target generation.

### Step 4: Storage Node Integrity Check

Run an offline integrity check on each storage node volume to verify chunk manifests and repository references:
```bash
for NODE in 1 2 3; do
  docker run --rm \
    -v spine_storage_${NODE}_data:/var/lib/spine/node-${NODE} \
    spine-storage-node:latest \
    /usr/local/bin/spine-storage-node --verify-integrity --data-dir /var/lib/spine/node-${NODE}
done
```

### Step 5: Sequenced Cluster Startup

Start services in dependency order:

1. **Start Core Cluster Infrastructure:**
   ```bash
   docker compose -f docker-compose.prod.yml up -d postgres coordinator storage-node-1 storage-node-2 storage-node-3
   ```
2. **Wait for Health Checks to Pass:**
   ```bash
   docker compose -f docker-compose.prod.yml ps
   # Verify that postgres, coordinator, storage-node-1, storage-node-2, storage-node-3 are (healthy)
   ```
3. **Start Application Control Plane & Ingress:**
   ```bash
   docker compose -f docker-compose.prod.yml up -d spine-api spine-frontend ingress
   ```

### Step 6: Readiness Gate & Consistency Verification

Execute the fail-closed readiness healthcheck:
```bash
curl -i http://127.0.0.1:3000/health/ready
```
**Required Response:**
- HTTP Status Code: `200 OK`
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

---

## 6. Audit Trail & Compliance Verification

Every Out-of-Band backup and restore operation must be recorded in the site operations log with:
1. Snapshot timestamp (UTC)
2. Fencing generation token
3. Storage node volume SHA-256 digests
4. Verification test output
5. Operator signature
