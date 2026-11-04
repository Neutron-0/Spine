# Spine Deployment Capacity & Storage Economics Model

## 1. Workload Sizing & Baseline Assumptions

This capacity model evaluates a medium-to-large organization hosting **200 GB+ of raw active user code and repositories** on Spine.

### Baseline Parameters
- **Raw User Code & Repositories:** 200 GB
- **Average Compression Ratio (Spine .spack BLAKE3 + Zstandard):** 1.8x reduction (~111 GB compressed base)
- **Active Monthly Growth Rate:** 15 GB / month
- **Associated Large File Storage (LFS) & Binaries:** 350 GB
- **Build Artifacts & Package Releases:** 150 GB
- **PostgreSQL Control Plane Metadata:** ~25 GB (indexes, commit metadata, audit events)
- **Total Raw Working Dataset:** ~636 GB

---

## 2. Storage Replication Matrix (1x, 2x, 3x Copies)

Spine's distributed architecture allows configurable replication durability policies ($R=1, R=2, R=3$):

| Tier | Policy | Usable User Data | Replicated Object Storage | DB + Indexes (WAL) | Operational Headroom (30%) | Total Storage Required |
|---|---|---|---|---|---|---|
| **Development / Single Node** | $R = 1$ | 636 GB | 611 GB | 35 GB | 194 GB | **840 GB** |
| **High Availability (Standard)** | $R = 2$ | 636 GB | 1,222 GB | 70 GB | 388 GB | **1,680 GB** (1.68 TB) |
| **Enterprise Multi-Zone Fault Tolerant** | $R = 3$ | 636 GB | 1,833 GB | 105 GB | 581 GB | **2,519 GB** (2.52 TB) |

### Headroom Rationale:
A 30% storage buffer is strictly enforced to prevent filesystem write lockouts during packfile repacking, concurrent git pushes, large LFS uploads, and temporary `.tar.gz` backup creation.

---

## 3. Compute, IOPS & Bandwidth Requirements

### Compute & Memory Sizing
- **Spine API & SSH Frontends (2x instances for HA):**
  - CPU: 4 vCPU per node (8 vCPU total)
  - RAM: 8 GB per node (16 GB total)
  - Role: Handles concurrent TLS termination, SSH connections, packfile stream decompression, JSON API parsing.
- **Spine Coordinator (3x nodes for Raft quorum):**
  - CPU: 2 vCPU per node
  - RAM: 4 GB per node
- **Spine Storage Nodes (3x nodes across Zone A, B, C):**
  - CPU: 4 vCPU per node
  - RAM: 16 GB per node (Page cache for high-frequency BLAKE3 object reads)
- **PostgreSQL Database (Primary + Read Replica):**
  - CPU: 4 vCPU
  - RAM: 16 GB (Shared buffers: 4 GB, Work mem: 64 MB)

### Storage IOPS & Throughput
- **Repository Write Path:** Sustained 3,000 IOPS per storage node (burst 10,000 IOPS during large repo migration or bulk push).
- **Read Throughput:** 250 MB/s per storage node for fast cloning.
- **Drive Tier Recommendation:** Direct-attached NVMe PCIe 4.0 or AWS EBS `gp3` (provisioned at 6,000 IOPS and 350 MB/s throughput).

---

## 4. Storage Media Economics & Cloud Cost Modeling

### Monthly Cloud Infrastructure Cost Estimate (AWS us-east-1 reference)

| Component | Specification | Quantity | Unit Monthly Cost | Total Monthly Cost |
|---|---|---|---|---|
| **API / Frontend Nodes** | `c6i.xlarge` (4 vCPU, 8 GB RAM) | 2 | \$122.64 | \$245.28 |
| **Coordinator Nodes** | `t4g.medium` (2 vCPU, 4 GB RAM) | 3 | \$24.53 | \$73.59 |
| **Storage Nodes** | `r6i.xlarge` (4 vCPU, 32 GB RAM) | 3 | \$183.96 | \$551.88 |
| **Storage Node EBS Volumes** | 1,000 GB `gp3` (6,000 IOPS, 350 MB/s) | 3 | \$112.00 | \$336.00 |
| **Managed PostgreSQL (RDS)** | `db.m6i.xlarge` Multi-AZ (4 vCPU, 16 GB) | 1 | \$379.60 | \$379.60 |
| **PostgreSQL EBS Storage** | 200 GB `io2` (3,000 IOPS) | 1 | \$155.00 | \$155.00 |
| **Object Backup Storage (S3)** | S3 Standard (3 TB backups + versioning) | 1 | \$69.00 | \$69.00 |
| **Network Egress** | 2 TB monthly data transfer | 1 | \$180.00 | \$180.00 |
| **Total Estimated Cloud Monthly Cost** | | | | **\$1,990.35 / month** |

### On-Premises / Bare Metal Economics Comparison
Deploying on 3x dedicated Dell PowerEdge R660 nodes (each with 32 cores, 128 GB RAM, and 4x 3.84 TB Enterprise NVMe U.2 SSDs in RAID-10):
- Capital Expenditure (CapEx): ~\$24,000 amortized over 36 months = **\$666.67 / month**.
- Colocation, Power & Bandwidth: ~**\$450.00 / month**.
- **Total Bare Metal Equivalent Cost:** **\$1,116.67 / month** (~44% cost savings over cloud at scale).
