# Spine Coordinated Backup Architecture

## 1. Scope & Objective

A Spine backup must capture a cryptographically consistent, point-in-time state across two decoupled systems:
1. **Relational Control Plane Metadata** (PostgreSQL).
2. **Repository Filesystem & Artifact Objects** (Storage Nodes / Local Storage Root).

Partial backups (e.g. taking a DB dump without matching storage objects, or vice versa) lead to dangling object pointers, broken commit references, or missing LFS blobs.

---

## 2. Backup Archive Format & Structure

The `spine-api backup create` command bundles storage objects, metadata snapshots, and cryptographic manifests into a compressed, streaming `.tar.gz` archive.

### Manifest Schema (`manifest.json`)
```json
{
  "backup_id": "7f9c2d1b-8e34-4b9a-a82f-0b2a8d4a9f31",
  "timestamp_nanos": 1793839200000000000,
  "software_version": "0.1.0",
  "schema_version": "0011_security_center",
  "total_files": 4512,
  "total_bytes": 104857600,
  "file_hashes": {
    "repos/owner/repo.spine/HEAD": "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9",
    "repos/owner/repo.spine/objects/pack/pack-001.spack": "5e884898da28047151d0e56f8dc6292773603d0d6aabbdd62a11ef721d1542d8",
    "database/snapshot.json": "7d793037a0760186574b0282f2f435e70ec0ec29e0004015094b7746190918b4"
  },
  "ssh_host_key_fingerprint": "SHA256:4t7a1cK4V2gW+Z9w+yR0t3kL7u8...",
  "integrity_status": "VERIFIED"
}
```

---

## 3. Coordinated Backup Execution Workflow

```
[Administrator / Cron Job]
           │
           ▼
[spine-api backup create --output /backups/spine_backup.tar.gz]
           │
           ├─► 1. Flush & Fsync in-flight VCS write journals
           ├─► 2. Dump PostgreSQL relational state (pg_dump -Fc or JSON snapshot)
           ├─► 3. Scan & compute SHA-256 for all files in /var/lib/spine/storage
           ├─► 4. Read SSH host key Ed25519 public fingerprint
           ├─► 5. Generate manifest.json with file hash table
           └─► 6. Stream tar.gz archive with manifest.json first, followed by payload
```

---

## 4. Integrity Verification Guarantees

1. **Pre-Restoration Integrity Check**:
   During restoration (`spine-api backup restore`), `manifest.json` is unpacked first. Every file extracted is hashed via SHA-256 and matched against `manifest.file_hashes`. If even a single byte differs or an unexpected file is present, restoration immediately halts with an error, preventing corrupt files from overwriting production data.
2. **Host Key Continuity**:
   The backup includes `ssh/host_key_ed25519` so restored instances present the identical host key fingerprint, ensuring SSH clients and Git automations do not experience host key mismatch errors.
