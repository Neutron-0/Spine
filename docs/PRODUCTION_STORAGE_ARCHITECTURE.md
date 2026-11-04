# Spine Production Storage Architecture & Directory Topology

## 1. Architectural Overview

Spine decouples persistent storage into distinct layers to guarantee durability, transactional safety, and non-blocking scale:

1. **Native Repository Storage (`repos/`)**:
   - Spine's native BLAKE3 object graph, loose objects, and `.spack` packfiles.
   - Authoritative transactional ref updates via two-phase journals (`journal/`).
2. **Artifact & Release Storage (`artifacts/`)**:
   - Release binaries, CI build outputs, Cargo crates, and Large File Storage (LFS) payloads.
   - Content-addressable storage hashed via SHA-256 (Git LFS) and BLAKE3 (Spine native).
3. **Control Plane Metadata (`postgres`)**:
   - User identities, SSH keys, repository permissions, issues, PRs, review comments, and security audit logs stored in relational PostgreSQL tables.
4. **Cluster State & Leases (`coordinator.state`)**:
   - Partition ownership epochs, node heartbeats, and replication leases managed by the Spine Coordinator.

---

## 2. Directory Layout & Mount Topology

In production environments, `/var/lib/spine` is mounted on dedicated persistent NVMe or high-IOPS block storage with `noatime` mount options.

```
/var/lib/spine/
├── storage/
│   ├── repos/
│   │   └── <owner_uuid_or_handle>/
│   │       └── <repo_uuid_or_name>.spine/
│   │           ├── HEAD
│   │           ├── config
│   │           ├── refs/
│   │           │   ├── heads/
│   │           │   └── tags/
│   │           ├── objects/
│   │           │   ├── loose/
│   │           │   └── pack/
│   │           └── journal/
│   ├── artifacts/
│   │   ├── lfs/
│   │   │   └── <sha256_prefix>/<sha256>
│   │   ├── releases/
│   │   └── packages/
│   │       └── cargo/
│   ├── runner_workspaces/
│   └── ssh/
│       ├── host_key_ed25519
│       └── host_key_ed25519.pub
├── backups/
└── coordinator.state
```

---

## 3. Storage Invariants & Durability Guarantees

1. **Append-Only Packfiles & Artifacts**:
   - `.spack` files and artifact blobs are immutable once written and verified.
   - Deletions only occur during scheduled garbage collection cycles after unreferenced grace periods (default: 14 days).
2. **Atomic Rename Semantics**:
   - All object writes, ref updates, and artifact ingestion write to temporary files on the same filesystem (`.tmp_*`) before atomic rename (`renameat2` or POSIX rename).
   - This prevents partial writes or zero-byte corrupted files during power loss or abrupt node termination.
3. **Filesystem Requirements**:
   - Storage volumes must support POSIX fsync and atomic directory operations (Linux ext4, XFS, or ZFS). Network filesystems (NFS) are prohibited for repository directories due to locking and sync latency hazards.
