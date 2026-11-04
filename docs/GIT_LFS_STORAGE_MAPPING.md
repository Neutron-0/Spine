# Spine Forge — Git LFS Storage Mapping & Deduplication

## 1. SHA-256 to BLAKE3 Identity Reconciliation

Git LFS mandates SHA-256 as its content-addressing algorithm for pointers and transfer manifests. Spine's internal distributed storage engine (`spine-artifact` and `spine-core`) uses BLAKE3 for content-addressed chunking and integrity verification.

```
+-----------------------------------------------------------+
| Git LFS Pointer (in Git Tree)                              |
|   oid sha256: 64e301d45536b2eec82ae888203bc472c40980692dd |
|   size: 5242880                                           |
+-----------------------------------------------------------+
                             │
                             ▼
+-----------------------------------------------------------+
| Spine Artifact Manifest (Stored in state.db & store)      |
|   artifact_id: 64e301d45536b2eec82ae888203bc472c40980692dd|
|   whole_object_hash: 64e301d4... (SHA-256)                |
|   total_size: 5242880                                     |
|   chunk_size: 4194304 (4 MiB)                             |
|   chunks:                                                 |
|     - index 0: offset 0, len 4194304, blake3: a8f9...    |
|     - index 1: offset 4194304, len 1048576, blake3: c3d1...|
+-----------------------------------------------------------+
```

---

## 2. Chunking & Local Storage Layout

Chunks are stored under `LocalArtifactStore` (`<storage_root>/artifacts/`):
- `chunks/<prefix>/<chunk_hash>`: Raw immutable block bytes, keyed by BLAKE3 hash.
- `manifests/<prefix>/<manifest_id>.json`: Canonical `ArtifactManifest` JSON descriptor.

### Deduplication:
- Identical 4 MiB chunks across different LFS objects or releases share the exact same on-disk chunk file.
- Manifests preserve distinct repository scoping, ensuring that deduplicated storage does not compromise repository access control boundaries.

---

## 3. Garbage Collection & Retention Fencing

`spine-artifact` GC collectors preserve:
1. Active LFS pointers reachable from any published Git reference in `.spine/refs/`.
2. Upload sessions currently in progress (within TTL window).
3. Manifests referenced by release assets or Cargo packages.
4. Grace period: verified LFS objects have a 24-hour grace period during which GC will not purge them even if the corresponding Git commit has not yet arrived via `git-receive-pack`.
