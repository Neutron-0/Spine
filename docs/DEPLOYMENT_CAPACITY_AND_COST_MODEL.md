# Spine Deployment Capacity, Storage Sizing, and Infrastructure Economics Model

**Milestone:** Spine Batch 20 — Final Deployment Readiness Closure  
**Date:** 2026-11-05  
**Target Enterprise Workload:** 200 GB+ Raw User Source Code with Git LFS, Build Artifacts, and PostgreSQL Metadata.

---

## 1. Disaggregated Mathematical Sizing Model

To eliminate ambiguities from blended multipliers, the storage requirements of a Spine Forge deployment are calculated using explicit disaggregated variables.

### 1.1. Independent Input Variables

| Variable | Description | Baseline Value | Units |
|---|---|---|---|
| $S_{\text{code}}$ | Raw active user source code and uncompressed git trees | $200.0$ | GB |
| $C_{\text{vcs}}$ | VCS delta-compression and packfile reduction factor | $1.80$ | Ratio |
| $S_{\text{lfs}}$ | Large File Storage (LFS) binary assets (compressed media/weights) | $350.0$ | GB |
| $S_{\text{art}}$ | Build releases, distribution packages, Cargo crate tarballs | $150.0$ | GB |
| $S_{\text{db\_raw}}$ | PostgreSQL relational records (users, PRs, issues, audit events) | $25.0$ | GB |
| $M_{\text{idx}}$ | Database index, WAL, and MVCC bloat multiplier | $1.40$ | Multiplier |
| $R$ | Cluster replication factor (durability policy) | $1, 2, 3$ | Factor |
| $H$ | Operational write headroom factor for repacking & backup buffers | $1.30$ | Multiplier (+30%) |
| $G_{\text{mo}}$ | Projected monthly raw data growth rate | $15.0$ | GB / month |

---

## 2. Mathematical Formulations & Derivations

### 2.1. Compressed VCS Storage ($S_{\text{vcs}}$)
Raw code is stored using BLAKE3 chunk deduplication, delta compression, and Zstandard compression in `.spack` archives:
$$S_{\text{vcs}} = \frac{S_{\text{code}}}{C_{\text{vcs}}} = \frac{200.0\text{ GB}}{1.80} \approx 111.11\text{ GB}$$

### 2.2. Effective Usable Object Storage ($S_{\text{obj\_usable}}$)
Total unique object storage across compressed VCS packfiles, LFS blobs, and packages:
$$S_{\text{obj\_usable}} = S_{\text{vcs}} + S_{\text{lfs}} + S_{\text{art}} = 111.11 + 350.0 + 150.0 = 611.11\text{ GB}$$

### 2.3. Database & WAL Storage ($S_{\text{db}}$)
PostgreSQL table data plus B-tree indexes, GIN search vectors, and WAL logs:
$$S_{\text{db}} = S_{\text{db\_raw}} \times M_{\text{idx}} = 25.0\text{ GB} \times 1.40 = 35.00\text{ GB}$$

### 2.4. Total Physical Replicated Storage ($S_{\text{total}}$)
The total provisioned disk capacity accounting for replication policy $R$ and operational headroom $H$:
$$S_{\text{total}}(R) = \Big( (S_{\text{obj\_usable}} \times R) + (S_{\text{db}} \times \min(R, 2)) \Big) \times H$$

*(Note: Database replication typically uses Primary + 1 Standby for $R \ge 2$, so database copies are capped at 2).*

---

## 3. Storage Replication Matrix

Applying the disaggregated equations to $R=1$, $R=2$, and $R=3$:

### Scenario A: Single-Node / Evaluation Profile ($R = 1$)
- Replicated Objects: $611.11 \times 1 = 611.11\text{ GB}$
- Database: $35.00 \times 1 = 35.00\text{ GB}$
- Subtotal: $646.11\text{ GB}$
- With 30% Headroom ($H = 1.30$):
  $$S_{\text{total}}(1) = 646.11\text{ GB} \times 1.30 = \mathbf{839.94\text{ GB}}\quad (\approx 840\text{ GB})$$

### Scenario B: High Availability Standard Profile ($R = 2$)
- Replicated Objects: $611.11 \times 2 = 1,222.22\text{ GB}$
- Database (Primary + Replica): $35.00 \times 2 = 70.00\text{ GB}$
- Subtotal: $1,292.22\text{ GB}$
- With 30% Headroom ($H = 1.30$):
  $$S_{\text{total}}(2) = 1,292.22\text{ GB} \times 1.30 = \mathbf{1,679.89\text{ GB}}\quad (\approx 1.68\text{ TB})$$

### Scenario C: Enterprise Multi-Zone Fault Tolerant Profile ($R = 3$)
- Replicated Objects: $611.11 \times 3 = 1,833.33\text{ GB}$
- Database (Multi-AZ Primary + Replica): $35.00 \times 2 = 70.00\text{ GB}$
- Subtotal: $1,903.33\text{ GB}$
- With 30% Headroom ($H = 1.30$):
  $$S_{\text{total}}(3) = 1,903.33\text{ GB} \times 1.30 = \mathbf{2,474.33\text{ GB}}\quad (\approx 2.47\text{ TB})$$

---

## 4. Growth Projection (12-Month Horizon)

Assuming monthly raw user code and binary growth $G_{\text{mo}} = 15.0\text{ GB/month}$:
- Annual raw growth: $15.0 \times 12 = 180.0\text{ GB}$.
- Compressed annual object growth: $\Delta S_{\text{obj\_usable}} = \frac{15.0 \times 12}{1.80} = 100.0\text{ GB}$.
- Annual database metadata growth: $\Delta S_{\text{db}} = 10.0\text{ GB} \times 1.4 = 14.0\text{ GB}$.
- Year 1 Total Physical Storage Required ($R = 3$):
  $$S_{\text{year1}}(3) = \Big( (711.11 \times 3) + (49.00 \times 2) \Big) \times 1.30 = (2,133.33 + 98.00) \times 1.30 = \mathbf{2,900.73\text{ GB}}\quad (\approx 2.90\text{ TB})$$

---

## 5. Cloud Infrastructure Cost Model (AWS us-east-1 Reference)

| Component | Specification | Quantity | Unit Price | Monthly Total |
|---|---|---|---|---|
| **API & SSH Frontends** | `c6i.xlarge` (4 vCPU, 8 GB RAM) | 2 | \$122.64 | \$245.28 |
| **Cluster Coordinators** | `t4g.medium` (2 vCPU, 4 GB RAM) | 3 | \$24.53 | \$73.59 |
| **Distributed Storage Nodes** | `r6i.xlarge` (4 vCPU, 32 GB RAM) | 3 | \$183.96 | \$551.88 |
| **Storage Node gp3 EBS** | 1,000 GB `gp3` (6,000 IOPS, 350 MB/s) | 3 | \$112.00 | \$336.00 |
| **Managed PostgreSQL RDS** | `db.m6i.xlarge` Multi-AZ (4 vCPU, 16 GB) | 1 | \$379.60 | \$379.60 |
| **PostgreSQL Dedicated EBS** | 200 GB `io2` (3,000 provisioned IOPS) | 1 | \$155.00 | \$155.00 |
| **Encrypted S3 Backups** | S3 Standard (3 TB encrypted `.spinebak`) | 1 | \$69.00 | \$69.00 |
| **Network Egress Bandwidth** | 2 TB monthly data transfer | 1 | \$180.00 | \$180.00 |
| **Total Cloud Hosting Cost** | | | | **\$1,990.35 / mo** |

### Bare-Metal Cost Comparison (3-Node Dedicated Cluster)
- Hardware: 3x Dell PowerEdge R660 (32 vCPU, 128 GB RAM, 4x 3.84 TB NVMe U.2 Enterprise SSDs).
- Amortized Hardware CapEx (36 months): \$666.67 / month.
- Datacenter Colocation, Power & 10 Gbps Transit: \$450.00 / month.
- **Total Bare-Metal Cost:** **\$1,116.67 / month** (**43.9% savings** compared to equivalent public cloud).
