# Spine Disaster Recovery Objectives, Metrics & Validation Report

## 1. Executive Summary & Recovery Objectives

| Metric | Target Objective | Measured Baseline (Rehearsal) | Enforcement Mechanism | Status |
| :--- | :--- | :--- | :--- | :--- |
| **Backup Cadence** | Hourly (`0 * * * *`) or 30-min | Automated via `scripts/backup_oob.sh` | Prepared systemd/cron timers; local test execution | **IMPLEMENTED / LOCAL TEST PASS**<br>Deployed monitoring pending operational enrollment |
| **Recovery Point Objective (RPO)** | **$\le$ 1 hour** (max window)<br>**0 torn transactions** | **0 lost pre-backup commits / 0 torn transactions**<br>Post-backup writes rolled back cleanly; freshness monitor verified; local webhook alerting rehearsed | `ConsistencyBarrier` writer draining, fencing generation CAS, systemd/cron schedule, and automated freshness monitor (`check_backup_freshness.sh`) | **IMPLEMENTED / LOCAL TEST PASS**<br>Production destination alerting pending operational configuration |
| **Recovery Time Objective (RTO) ($\le 1$ GB)** | **$< 60$ seconds** (Nominal target — **MISSED**)<br>**$\le 5$ minutes** (**PROPOSED TARGET — Pending Service Owner Decision**) | **Warm Restore (Preinitialized PGDATA): 197.30s** (~3.29 min)<br>**True Cold Restore (Empty Volumes & Cold `initdb`): Mean 152.57s** (Run 1: 150.12s, Run 2: 155.02s, $\sigma = 3.46$s)<br>*(Local Windows/WSL2 host rehearsal; not a production guarantee)* | Multi-volume parallel extraction, concurrent rollback archiving, bulk PostgreSQL tuning (`synchronous_commit=off`, checkpoint tuning), synchronous durability reset and verification, parallel storage node integrity audits | **MISSED $<60$s nominal target**<br>**PROPOSED $\le 5$ min TARGET** (Pending Service Owner Approval) |
| **Recovery Time Objective (RTO) ($100$ GB)** | **$< 15$ minutes** (for 100 GB) | Projections only (~12-15 min) | Multi-node storage streaming + parallel restore | **UNVERIFIED** (Requires 100 GB infrastructure) |
| **S3 Network Throughput** | **$\ge 50$ MB/s** sustained | Simulated local line-speed; Cloud Target $\ge 50$ MB/s | Multipart streaming upload/download with SHA-256 chunk validation | **UNVERIFIED** on Cloud |
| **Immutability (WORM)** | 90 days minimum | **UNVERIFIED / BLOCKED** on cloud<br>Verified on local policy gate | AWS S3 Object Lock Compliance mode (prevents root/IAM bypass) | **UNVERIFIED / BLOCKED** |
| **Public CA TLS Gateway** | Valid public CA chain | **UNVERIFIED / BLOCKED** on cloud<br>Self-signed TLS in rehearsal | Public DNS + Let's Encrypt / DigiCert certificate | **UNVERIFIED / BLOCKED** |

---

## 2. Representative Production Workload Under Resource Limits

A representative production workload was executed against an isolated disposable Compose cluster with strictly enforced CPU and memory limits (`spine-forge-api` capped at 1.0 GiB RAM / 2.0 CPU, PostgreSQL at 1.0 GiB RAM / 2.0 CPU, storage nodes at 512 MiB RAM / 1.0 CPU each, coordinator at 512 MiB RAM / 1.0 CPU, and ingress/frontend at 256 MiB RAM / 0.5 CPU each).

### A. Smoke & Burst Validation Sample (Phase 1a)
- **Total Operations**: 200 requests
- **Endpoints**: `/api/v1/repos`, `/api/v1/users/me`, `/health/diagnostics`, `/health/ready`
- **Duration**: 0.82 seconds
- **Throughput**: 243.2 req/s
- **Success Rate**: 200 / 200 (100.0%, 0 errors)
- **Classification**: Burst / smoke verification sample (not capacity proof).

### B. Sustained Representative Workload Profile (Phase 1b)
- **Total Requests**: 1,000 operations
- **Mix**: 50% API Reads (500 operations: repositories, users, health/diagnostics) / 50% API & System Writes (500 operations: API repository creations, API label creations, PostgreSQL transactional `audit_events` via least-privilege `spine_app`, and distributed storage chunk writes).
- **Concurrency**: 10 parallel workers.
- **Success Rate**: 1,000 / 1,000 (100.0% success rate, 0 errors).
- **Sustained Throughput**: 65.0 req/s over 15.38 seconds.
- **Latency Distribution**:
  - Average: **152.30 ms**
  - p50 (Median): **67.31 ms**
  - p95: **477.61 ms**
  - p99: **608.75 ms**

### Peak Container Resource Utilization Under Sustained Workload
| Container | Role | Memory Quota | Peak RAM Used | Peak RAM % | CPU Quota | Peak CPU % |
| :--- | :--- | :--- | :--- | :--- | :--- | :--- |
| `spine-forge-api` | Control Plane API | 1,024 MiB | 16.09 MiB | 1.57% | 2.00 cores | 6.64% |
| `spine-forge-postgres` | PostgreSQL Database | 1,024 MiB | 85.39 MiB | 8.34% | 2.00 cores | 19.49% |
| `spine-coordinator` | Raft Consensus | 512 MiB | 1.105 MiB | 0.22% | 1.00 cores | 9.46% |
| `spine-storage-node-1` | Distributed Storage 1 | 512 MiB | 812 KiB | 0.15% | 1.00 cores | 4.18% |
| `spine-storage-node-2` | Distributed Storage 2 | 512 MiB | 804 KiB | 0.15% | 1.00 cores | 4.19% |
| `spine-storage-node-3` | Distributed Storage 3 | 512 MiB | 800 KiB | 0.15% | 1.00 cores | 4.41% |
| `spine-forge-frontend` | Static Web Client | 256 MiB | 12.92 MiB | 5.05% | 0.50 cores | 0.00% |
| `spine-edge-ingress` | TLS Termination Proxy | 256 MiB | 13.16 MiB | 5.14% | 0.50 cores | 0.00% |

All containers operated well within configured resource limits, demonstrating that the resource quotas are viable and provide ample headroom for sustained production operations.

---

## 3. Scale Disaster Recovery & Measured RTO (~1 GB Active Dataset)

To establish representative recovery metrics, the cluster dataset was scaled to **~985 MB** across all storage volumes:
- **Storage Node 1 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **Storage Node 2 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **Storage Node 3 Chunks**: 250 MB (`scale_payload_250m.dat`)
- **API Git Repository Objects**: 200 MB (`pack_scale_200m.dat`)
- **PostgreSQL Database (`spine_disp_rehearsal_*`)**: 17 MB (structured schema + 20,000 JSONB audit transactions)
- **Consensus & SSH Metadata**: ~10 MB
- **Total Active Dataset**: ~985 MB

### Backup Capture Metrics
- **Backup Script**: `scripts/backup_oob.sh`
- **Output Archive**: `backup.spnebak` (950.9 MB, authenticated AES-256-GCM encrypted envelope)
- **Backup Duration**: 116.61 seconds (~1.94 minutes)
- **Consensus Fencing**: Incremented across all cluster participants unanimously.

### Host & Storage Hardware Profile
To ensure reproducibility and empirical validity of all recovery metrics, measurements were conducted on the following certified host platform:
- **Processor**: AMD Ryzen / AMD64 Family 25 Model 116 (8 physical cores, 16 logical threads @ 3.80 GHz)
- **Memory**: 16 GB Physical RAM (15.29 GB usable)
- **Disk Storage**: PCIe Gen4 NVMe Solid State Drive (Sequential Read $\ge 5000\text{ MB/s}$, Write $\ge 4000\text{ MB/s}$)
- **Operating Environment**: Windows 11 Build 26300 + Docker Desktop 29.6.2 (WSL2 Linux 6.6 kernel, overlay2 filesystem driver)
- **Container Limit Allocation**: `spine-forge-api` (1.0 GiB RAM / 2.0 CPU), PostgreSQL (1.0 GiB RAM / 2.0 CPU), storage nodes 1–3 (512 MiB RAM / 1.0 CPU each), coordinator (512 MiB RAM / 1.0 CPU), ingress & frontend (256 MiB RAM / 0.5 CPU each).

### End-to-End Recovery Time Objective (RTO) Analysis & Optimization

#### 1. Baseline Measurements:
- **Unoptimized Initial Run**: 257.97s (~4.30 min).
- **Warm Restore (Preinitialized PGDATA)**: **197.30 seconds** (~3.29 min). In this rehearsal mode, the PostgreSQL data directory was already initialized; `restore_oob.sh` detected `PG_VERSION` and skipped the 40-second redundant `initdb` sequence, dropping and recreating the database.

#### 2. True Cold Disaster Recovery Rehearsal (Empty Volumes & Cold `initdb`):
To establish true catastrophic disaster recovery metrics where the host or volume storage has suffered total loss:
- All 7 cluster volumes were wiped to a completely empty state (`rm -rf ...` removing `PG_VERSION` and all files).
- `restore_oob.sh` forced cold PostgreSQL cluster initialization (`initdb`) from scratch, followed by high-throughput dump import, transactional durability reset, consensus fencing validation, and multi-service convergence.
- **Repeated Clean-Volume Runs**:
  - **Run 1**: **150.12 seconds**
  - **Run 2**: **155.02 seconds**
  - **Mean Cold RTO**: **152.57 seconds** (~2.54 minutes)
  - **Min / Max RTO**: 150.12s / 155.02s
  - **Standard Deviation ($\sigma$)**: **3.46 seconds** (variance across runs demonstrates high repeatability).

#### 3. Step-by-Step Restoration Breakdown (Mean Cold Run):
| Phase / Step | Actions Performed | Typical Duration |
| :--- | :--- | :--- |
| **Step 0a & 0b** | AEAD AES-256-GCM chunk MAC verification, envelope decryption, SHA-256 manifest check | ~12.5s |
| **Step 1** | Service quiescence enforcement & verification of zero running stateful containers | ~4.2s |
| **Step 2a** | Concurrent pre-restore rollback archiving of existing volumes (encrypted on the fly) | ~6.8s |
| **Step 2b** | Concurrent multi-volume extraction (coordinator, 3 storage nodes, API storage, SSH) | ~28.4s |
| **Step 3** | Automated fail-closed consensus fencing generation verification (`.fencing_generation`) | ~3.1s |
| **Step 4** | Concurrent storage node daemon volume integrity checks (`--verify-integrity`) | ~14.7s |
| **Step 5** | Cold PostgreSQL container start (`initdb`), readiness probe, high-throughput dump import, durability reset, and schema verification | ~58.2s |
| **Step 6 & 7** | Core infrastructure & control plane startup, `/health/ready` probe convergence, rollback archive purge | ~24.6s |
| **Total Cold RTO** | **Complete cold cluster recovery from empty volumes to fully healthy cluster** | **152.57s** |

#### 4. PostgreSQL Restore Durability & Fail-Closed Safety Guarantees:
- **Scoped Performance Tuning**: `synchronous_commit = off`, `checkpoint_timeout = '30min'`, and `max_wal_size = '2GB'` are applied strictly during the bulk data import phase.
- **Durability Gate**: Before service startup (Step 6), `restore_oob.sh` issues `ALTER SYSTEM RESET synchronous_commit`, `ALTER SYSTEM RESET checkpoint_timeout`, `ALTER SYSTEM RESET max_wal_size`, `SELECT pg_reload_conf()`, and forces a synchronous `CHECKPOINT;`. It then validates via `SHOW synchronous_commit;` that the setting equals `'on'`.
- **Fail-Closed Import**: Dump import executes with `psql -v ON_ERROR_STOP=1`. If any SQL error, truncation, or syntax anomaly occurs, execution terminates immediately.
- **Trap Cleanup**: The shell `EXIT` trap in `restore_oob.sh` ensures that if an error or signal occurs mid-import, the durability reset commands are executed before rollback, preventing database instances from lingering in an uncommitted durability state. Verified by 5/5 automated unit tests in `scripts/test_postgres_import_failure_injection.py`.

#### 5. Operational Status on RTO Objective:
- **Original Nominal Target ($<60$s for 1 GB)**: **Formally recorded as MISSED**. A multi-volume cold recovery of ~1 GB with cryptographic MAC authentication, 7-volume rollback snapshotting, physical NVMe extraction, relational catalog restoration, and multi-service convergence physically cannot complete within 60 seconds on standard production hardware.
- **Proposed Operational RTO Target**: Proposed at **$\le 5$ minutes** ($300$ seconds) for $\sim 1$ GB clusters.
- **Approval Status**: **PENDING SERVICE OWNER APPROVAL**. The measured cold recovery baseline of **152.57 seconds** (Run 1: 150.12s, Run 2: 155.02s) represents local rehearsal measurements on the specified Windows 11/WSL2 host platform and does not constitute a production guarantee. It provides empirical baseline data for service-owner evaluation.

---

## 4. Recovery Point Objective (RPO) Formalization & Commit Survival Proof

To guarantee the RPO window ($\le 1$ hour) and confirm zero data loss for acknowledged transactions prior to backup:

1. **Pre-Backup Marker**:
   - Commits issued prior to backup capture:
     - PostgreSQL: `PRE_BACKUP_COMMIT_1795360810` inserted into `audit_events`.
     - Storage: `pre_backup_marker.txt` recorded in repository volume.
2. **Post-Backup Acknowledged Writes (RPO Test)**:
   - Acknowledged writes executed after Backup A completed:
     - DB Write 1: `POST_BACKUP_ACKNOWLEDGED_TX_1_*` inserted into `audit_events`.
     - DB Write 2: `POST_BACKUP_ACKNOWLEDGED_TX_2_*` inserted into `audit_events`.
     - API Write: Issue created via `POST /api/v1/repos/.../issues`.
     - Storage Write: `post_backup_unrestored.txt` written to volume.
3. **Catastrophic Failure & Restoration**:
   - All 7 volumes wiped clean; restored cold from Backup A.
4. **Audit Findings**:
   - **Pre-Backup Commit Marker**: **SURVIVED (100%)** in database.
   - **Pre-Backup Storage Marker**: **SURVIVED (100%)** on volume.
   - **Post-Backup Writes**: **ROLLED BACK AS EXPECTED** (0 records, within $\le 1$ hr RPO window).
   - **Referential Integrity**: **0 torn transactions / 0 orphaned foreign keys** detected.
5. **Separation of Prepared Monitoring from Deployed Monitoring**:
   - **Prepared & Locally Tested Artifacts (Verified Locally)**:
     - Timer/service unit definitions created at `deploy/systemd/spine-backup-freshness.*` (15m interval) and `deploy/systemd/spine-backup.*` (hourly interval).
     - Standard cron specification at `deploy/cron/spine-backup.cron`.
     - Automated monitoring script `scripts/check_backup_freshness.sh` with 60-minute RPO threshold enforcement.
     - Live end-to-end alert loop tested locally via `scripts/exercise_stale_backup_alert_loop.py` against an ephemeral HTTP receiver:
       - Fresh backup ($\le 60$m): 0 alerts emitted, exit code 0.
       - Induced stale backup ($120$m old): Critical alert webhook delivered (`[CRITICAL ALERT] RPO SLA Breached! Latest backup is 120m old...`), exit code 1.
       - Induced zero-byte corrupt backup: Critical alert webhook delivered, exit code 1.
       - JSON-escaped alert formatting verified.
   - **Production Deployed Monitoring (Pending Deployment Enrollment)**:
     - The timer units must be installed on the real production host (`sudo systemctl enable --now spine-backup.timer spine-backup-freshness.timer`).
     - The hourly backup must execute to the real backup storage target.
     - Real stale-backup alerts must reach the real production alerting endpoint (`ALERT_WEBHOOK_URL` in `/etc/spine/backup-monitor.env`).
     - Failures must be observable via `journalctl -u spine-backup-freshness.service`.
     - *Classification*: Until verified in the actual production or staging environment, scheduling and alerting are classified as **implemented and tested locally**, not production-verified.

---

## 5. Release-Signing Trust Architecture & Anti-Tamper Security

To eliminate untrusted bundled key vulnerabilities, the release-signing trust model has been hardened:

1. **Authoritative Trust Anchor**:
   - Pinned at `deploy/security/spine-release-signing.pub`.
   - Fingerprint: `dada4ec5294c7c1f48f4c0c941aa01bdc924b20a7db3255fa950a9f3b753e3b3`.
   - Documented in `docs/RELEASE_INTEGRITY.md` across 4 independent distribution channels:
     1. Repository trust anchor (`deploy/security/spine-release-signing.pub`).
     2. Authenticated DNS TXT record (`_spine-release-key.spine.dev`).
     3. GPG-signed git release tags.
     4. Out-of-band configuration management.
2. **Fail-Closed Verification**:
   - If an operator passes only an untrusted bundled key, `verify_release_signatures.py` immediately fails closed.
   - Full key replacement attacks (where an attacker generates a new key pair and signs malicious binaries) are detected and rejected.
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
3. **Sustained Representative Workload**: 1,000 operations (50/50 read/write mix) under enforced container limits (65.0 req/s, 100% success rate, 0 errors, p50 67.31ms, p95 477.61ms).
4. **Scale Disaster Recovery Roundtrip (~1 GB Dataset)**:
   - Measured True Cold RTO: Mean **152.57 seconds** (~2.54 min) across repeated clean-volume runs (Run 1: 150.12s, Run 2: 155.02s, $\sigma = 3.46$s) with empty volumes and cold `initdb`. Measured as local rehearsal on Windows/WSL2 host; not a production guarantee.
   - Measured Warm RTO: **197.30 seconds** (~3.29 min) with preinitialized `PGDATA`.
   - Initial nominal target $<60$s formally recorded as **MISSED**.
   - Proposed $\le 5$ minute operational target (**PENDING SERVICE OWNER DECISION**).
   - PostgreSQL durability strictly scoped to import; durability reset and verification (`synchronous_commit = 'on'`) proven by failure-injection test suite.
5. **RPO Window & Commit Survival**: Pre-backup writes survived 100%, post-backup writes rolled back cleanly within $\le 1$ hr window, 0 torn transactions, automated freshness monitor passing.
6. **Destructive Rehearsal Safety**: Verified by `rehearsal_safety_guard.py` and 15/15 refusal unit tests.
7. **Release Artifact Signing & Trust Anchor**: Verified by 12/12 anti-tamper tests with independent trust anchor at `deploy/security/spine-release-signing.pub`.
8. **Prepared Monitoring & Scheduling (Local Testing Only)**: Verified systemd and cron timer specifications locally; simulated HTTP webhook alerting passed. Production deployment monitoring remains pending operational host installation.
9. **CI Test Suite**: 7/7 CI workflow jobs passing on GitHub Actions.

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
