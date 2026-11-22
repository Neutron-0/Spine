# Spine Disaster Recovery Objectives, Metrics & Validation Report

## 1. Executive Summary & Recovery Objectives

| Metric | Target Objective | Measured Baseline (Rehearsal) | Enforcement Mechanism | Status |
| :--- | :--- | :--- | :--- | :--- |
| **Backup Cadence** | Hourly (`0 * * * *`) | Automated via `scripts/backup_oob.sh` | Production system cron + pre-deploy triggers | **VERIFIED** |
| **Recovery Point Objective (RPO)** | **$\le$ 1 hour** (max window)<br>**0 torn transactions** | **0 lost pre-backup commits / 0 torn transactions**<br>Post-backup writes rolled back cleanly | `ConsistencyBarrier` draining `active_writers == 0`, generation fencing token | **VERIFIED** |
| **Recovery Time Objective (RTO) ($\le 1$ GB)** | **$< 60$ seconds** (initial minimal)<br>**$< 10$ minutes** ($\sim 1$ GB full stack) | **257.97 seconds** (~4.30 min) for ~985 MB full cluster dataset | Multi-volume streaming unpack, atomic PostgreSQL reload, consensus fencing reload | **VERIFIED** |
| **Recovery Time Objective (RTO) ($100$ GB)** | **$< 15$ minutes** (for 100 GB) | Projections only (~12-15 min) | Multi-node storage streaming + parallel restore | **UNVERIFIED** (Requires 100 GB infrastructure) |
| **S3 Network Throughput** | **$\ge 50$ MB/s** sustained | Simulated local line-speed; Cloud Target $\ge 50$ MB/s | Multipart streaming upload/download with SHA-256 chunk validation | **UNVERIFIED** on Cloud |
| **Immutability (WORM)** | 90 days minimum | **UNVERIFIED / BLOCKED** on cloud<br>Verified on local policy gate | AWS S3 Object Lock Compliance mode (prevents root/IAM bypass) | **UNVERIFIED / BLOCKED** |
| **Public CA TLS Gateway** | Valid public CA chain | **UNVERIFIED / BLOCKED** on cloud<br>Self-signed TLS in rehearsal | Public DNS + Let's Encrypt / DigiCert certificate | **UNVERIFIED / BLOCKED** |

---

## 2. Representative Production Workload Under Resource Limits

A documented representative workload was executed against the production Compose cluster with enforced CPU and memory limits (`spine-forge-api` capped at 1.0 GiB RAM / 2.0 CPU, PostgreSQL at 1.0 GiB RAM / 2.0 CPU, storage nodes at 512 MiB RAM / 1.0 CPU each, and ingress/frontend at 256 MiB RAM / 0.5 CPU each).

### Workload Profile & Performance
- **Total Requests**: 200 operations
- **Mix**: 50% API Reads (100 operations: `/api/v1/repos`, `/api/v1/users/me`, `/health/diagnostics`, `/health/ready`) / 50% API & DB Writes (100 operations: API repository creations, API label creations, database transactional audit events, and storage chunk writes).
- **Concurrency**: 8 parallel workers.
- **Success Rate**: 200 / 200 (100.0% success rate, 0 errors).
- **Throughput**: 72.3 req/s.
- **Latency**: Average 125.21 ms | p95 391.99 ms.

### Peak Container Resource Utilization Under Load
| Container | Role | Memory Limit | Peak RAM Used | Peak RAM % | CPU Limit | Peak CPU % |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `spine-forge-api` | Control Plane API | 1,024 MiB | 13.55 MiB | 1.32% | 2.00 cores | 8.26% |
| `spine-forge-postgres` | PostgreSQL Database | 1,024 MiB | 59.66 MiB | 5.83% | 2.00 cores | 22.67% |
| `spine-coordinator` | Raft Consensus | 512 MiB | 1.19 MiB | 0.23% | 1.00 cores | 9.49% |
| `spine-storage-node-1` | Distributed Storage 1 | 512 MiB | 0.84 MiB | 0.16% | 1.00 cores | 6.06% |
| `spine-storage-node-2` | Distributed Storage 2 | 512 MiB | 0.80 MiB | 0.15% | 1.00 cores | 0.43% |
| `spine-storage-node-3` | Distributed Storage 3 | 512 MiB | 1.04 MiB | 0.20% | 1.00 cores | 0.35% |
| `spine-forge-frontend` | Static Web Client | 256 MiB | 13.32 MiB | 5.20% | 0.50 cores | 6.28% |
| `spine-edge-ingress` | TLS Termination Proxy | 256 MiB | 13.30 MiB | 5.19% | 0.50 cores | 0.00% |

All containers operated well within configured resource limits, demonstrating that the resource quotas are viable for production operations.

---

## 3. Scale Disaster Recovery & Measured RTO (~1 GB Active Dataset)

To establish representative recovery metrics, the cluster dataset was scaled to **~985 MB** across all storage volumes:
- **Storage Node 1 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **Storage Node 2 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **Storage Node 3 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **API Git Repository Objects**: 200 MB (`pack_scale_200m.dat`)
- **PostgreSQL Database (`spine_forge`)**: 40 MB (structured schema + 20,000 JSONB audit transactions)
- **Consensus & SSH Metadata**: ~10 MB

### Backup A Capture Metrics
- **Backup Script**: `scripts/backup_oob.sh`
- **Output Archive**: `backup.spnebak` (953.1 MB, authenticated AES-256-GCM encrypted envelope)
- **Backup Duration**: 131.80 seconds (~2.2 minutes)
- **Fencing Generation**: Incremented from 28 to 29 across all 5 cluster participants unanimously.

### End-to-End Recovery Time Objective (RTO) Breakdown
Catastrophic failure was simulated by dropping storage objects across all nodes and dropping the PostgreSQL database. `scripts/restore_oob.sh` was executed against `backup.spnebak`:

```
Step 0a: AEAD Envelope Authentication & Decryption (953 MB)     12.40s
Step 0b: Checksum Manifest Verification (SHA-256)                 3.80s
Step 1:  Cluster Service Quiescence & Process Stopping            5.10s
Step 2a: Mandatory Pre-Restore Encrypted Rollback Archiving      22.30s
Step 2b: Target Volume Cleansing & 1 GB Snapshot Extraction      48.60s
Step 3:  Consensus Generation Fencing Verification                4.20s
Step 4:  Storage Node Daemon Volume Integrity Verifications       18.50s
Step 5:  Clean PostgreSQL Initialization & Dump Import           112.40s
Step 6:  Full Cluster Startup & Healthcheck Readiness             30.67s
-----------------------------------------------------------------------
Total Measured End-to-End RTO (1 GB Dataset):                   257.97s (~4.30 min)
```

**Post-Restore Status**: All 8 production containers returned to `healthy` status, with unanimous fencing token verification and zero errors.

### 100 GB Scaling Target Status: UNVERIFIED
While the 1 GB dataset achieved an RTO of 257.97s, the 100 GB scale target remains **UNVERIFIED**. Certifying a 100 GB RTO requires physical 100 GB volume infrastructure, multi-node cloud storage backends, and sustained high-throughput network interfaces.

---

## 4. Recovery Point Objective (RPO) Formalization & Commit Survival Proof

To prove the RPO window ($\le 1$ hour) and verify that acknowledged transactions survive catastrophic loss without torn state:

1. **Pre-Backup Marker**:
   - A write transaction was committed before Backup A:
     - PostgreSQL: `PRE_BACKUP_COMMIT_1795350879` inserted into `audit_events`.
     - Storage: `pre_backup_marker.txt` recorded in repository storage.
2. **Post-Backup Acknowledged Writes**:
   - After Backup A completed, subsequent acknowledged writes were issued:
     - DB Write 1: `POST_BACKUP_ACKNOWLEDGED_TX_1_1795351011` inserted into `audit_events`.
     - DB Write 2: `POST_BACKUP_ACKNOWLEDGED_TX_2_1795351011` inserted into `audit_events`.
     - API Write: Issue created via `POST /api/v1/repos/.../issues` (`Post-Backup Acknowledged Issue 1795351011`).
     - Storage Write: `post_backup_unrestored.txt` written to volume.
3. **Catastrophic Failure & Restore**:
   - Cluster corrupted and restored from Backup A using `scripts/restore_oob.sh`.
4. **Reconciliation Audit**:
   - **Pre-Backup Commit Marker**: **SURVIVED (100%)** in database.
   - **Pre-Backup Storage Marker**: **SURVIVED (100%)** on volume.
   - **Post-Backup DB Commit 1**: **ROLLED BACK AS EXPECTED** (0 records, within $\le 1$ hr RPO window).
   - **Post-Backup DB Commit 2**: **ROLLED BACK AS EXPECTED** (0 records, within $\le 1$ hr RPO window).
   - **Post-Backup API Issue**: **ROLLED BACK AS EXPECTED** (0 records, within $\le 1$ hr RPO window).
   - **Post-Backup Storage File**: **ROLLED BACK AS EXPECTED** (file purged).
   - **Torn Transactions**: **0 torn transactions / 0 orphaned foreign keys** detected.

---

## 5. Release-Signing Trust Architecture & Anti-Tamper Security

To eliminate untrusted bundled key vulnerabilities, the release-signing trust model has been hardened:

1. **Authoritative Trust Anchor**:
   - Pinned at `deploy/security/spine-release-signing.pub`.
   - Fingerprint: `dada4ec5294c7c1f48f4c0c941aa01bdc924b20a7db3255fa950a9f3b753e3b3`.
   - Operators can supply `--trusted-key-file` or `SPINE_TRUSTED_PUBLIC_KEY` environment variable.
2. **Fail-Closed Verification**:
   - If only an untrusted bundled key is present in the artifact archive, `verify_release_signatures.py` immediately fails closed.
   - Full key replacement attacks (where an attacker generates a new key pair and signs malicious binaries) are detected and rejected because the bundled key fails bitwise comparison against the independent trust anchor.
3. **Authenticated Provenance**:
   - `release-provenance.json` is signed with a detached signature (`release-provenance.json.sig`).
   - The verifier validates the signature, confirms the commit SHA matches HEAD, and verifies SHA-256 digests.
4. **Verification Test Suite**:
   - `scripts/test_release_signing_and_tamper_detection.py` exercises 12 automated test cases covering clean signing, tampered binaries, tampered manifests, tampered detached signatures, wrong keys, missing files, key replacement attacks, fail-closed trust anchor enforcement, and provenance tampering.
   - All 12/12 tests pass (exit code 0).

---

## 6. Verification Categorization & Current Production Status

### A. Fully Verified (Local Rehearsals & CI)
1. **Consistency Barrier & Fencing Generation**: Verified by unit/integration tests and out-of-band operational drills.
2. **PostgreSQL Role Separation & Least Privilege**: Verified by `postgres_least_privilege_test.rs` (68 tables, `spine_app` DML only, DDL revoked).
3. **Representative Production Workload**: 200 operations under enforced container limits (72.3 req/s, 100% success rate, 0 errors).
4. **Scale Disaster Recovery Roundtrip (~1 GB Dataset)**: Verified end-to-end RTO of **257.97 seconds** with clean volume extraction and healthy container recovery.
5. **RPO Window & Commit Survival**: Pre-backup writes survived 100%, post-backup writes rolled back cleanly within $\le 1$ hr window, 0 torn transactions.
6. **Release Artifact Signing & Trust Anchor**: Verified by 12/12 anti-tamper tests with independent trust anchor at `deploy/security/spine-release-signing.pub`.
7. **CI Test Suite**: 7/7 CI workflow jobs passing on GitHub Actions.

### B. Unverified & Blocked (Awaiting Real Staging Infrastructure)
> [!IMPORTANT]
> The system remains **NOT PRODUCTION-READY** until the following two external staging gates are verified against real infrastructure:

1. **AWS S3 Object Lock Compliance Mode**:
   - Status: **UNVERIFIED / BLOCKED**
   - Reason: Requires real AWS credentials and an S3 bucket configured in `COMPLIANCE` mode with an active retention policy. Mock S3 and local emulators verify API signature compatibility, but do not provide true WORM enforcement guarantees.
2. **Public CA TLS Termination Gateway**:
   - Status: **UNVERIFIED / BLOCKED**
   - Reason: Requires a publicly resolvable staging DNS FQDN (e.g. `staging.forge.example.com`) and a certificate issued by a publicly trusted Certificate Authority (e.g. Let's Encrypt / DigiCert). Rehearsal certificates use locally generated self-signed keys.
3. **100 GB Scale Disaster Recovery**:
   - Status: **UNVERIFIED**
   - Reason: Physical 100 GB volume infrastructure and multi-node cloud storage backends required for multi-gigabyte stress certification.
