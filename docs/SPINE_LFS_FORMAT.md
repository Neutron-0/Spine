# SPINE LFS FORMAT SPECIFICATION

## 1. Motivation

Storing multi-megabyte and multi-gigabyte binary objects directly in repository packfiles degrades performance during clone, fetch, diff, and memory-mapped packfile operations. 

Spine LFS replaces large files in the working directory and repository tree objects with compact text pointers pointing to the content-addressed artifact store.

---

## 2. Pointer Format (`spine-lfs-v1`)

Spine LFS pointers are formatted as clean, deterministic UTF-8 text strings:

```
spine-lfs-v1 <manifest_oid> <size_bytes> <object_hash>
```

### Multi-Line Canonical Variant
Spine also supports a canonical key-value formatted pointer:

```
version https://spine.build/spec/lfs/v1
oid spine-blake3:<manifest_oid>
size <size_bytes>
hash <object_hash>
```

### Fields
- `manifest_oid`: 64-character hexadecimal BLAKE3 identifier of the `ArtifactManifest`.
- `size_bytes`: Decimal representation of the exact uncompressed payload size.
- `object_hash`: 64-character hexadecimal BLAKE3 digest of the complete reconstructed payload.

Pointers are typically under 200 bytes. When committed into Spine trees, commits, and packfiles, they consume negligible storage while preserving exact cryptographic provenance.

---

## 3. Tracking Rules Configuration (`.spinelfs`)

Large file tracking rules are stored in the root of the repository in `.spinelfs`:

```ini
# Spine Large File Storage (LFS) Tracking Rules
*.psd
*.zip
*.tar.gz
*.mp4
*.onnx
models/*
assets/textures/**
```

Rules support standard glob patterns:
- Wildcard extensions (`*.psd`, `*.iso`)
- Prefix directories (`models/*`)
- Recursive subtrees (`assets/textures/**`)
- Exact relative paths (`bin/compiler_backend.exe`)

---

## 4. Worktree Interception Lifecycle

```
[Developer File on Disk]
         │
         ▼
[spine add path/to/large.psd]
         │
         ├── Matches .spinelfs pattern?
         │         │
         │         ├─► YES:
         │         │     1. Stream & chunk file into .spine/artifacts/chunks/
         │         │     2. Generate & store ArtifactManifest in .spine/artifacts/manifests/
         │         │     3. Construct LfsPointer string
         │         │     4. Stage LfsPointer text as standard Blob into Spine Index
         │         │
         │         └─► NO:
         │               Stage file directly into loose objects / packfile
         ▼
[spine commit]
Commit tree points to compact LfsPointer blob.
```
