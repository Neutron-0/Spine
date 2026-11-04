# SPINE ARTIFACT STORAGE ARCHITECTURE

## 1. Unified Content-Addressed Philosophy

Spine implements a single, unified, content-addressed artifact storage engine (`spine-artifact`) that powers:
1. **Spine LFS**: Large file versioning avoiding packfile bloat.
2. **Forge Releases**: Published releases and attached release binary assets.
3. **Generic Artifacts**: Build outputs, binary tars, container images, and dataset models.
4. **Package Registries**: Ecosystem registries starting with native Cargo crate packages.

Instead of fragmenting storage across disparate backends or wrapping external Git LFS tools, Spine establishes a unified content-addressed foundation:
- **Zero External Dependencies**: Strictly no Git, Git LFS, or `libgit2` binaries or libraries.
- **Content Addressed Chunks**: Payload files are deterministically chunked into bounded blocks (default 4 MiB) identified by BLAKE3 digests.
- **Cryptographic Manifests**: Every stored artifact is backed by an `ArtifactManifest` specifying total payload size, chunk count, whole-object BLAKE3 digest, and indexed list of chunk digests with offsets and lengths.
- **Deterministic Deduplication**: Multiple artifacts, releases, or LFS files sharing identical chunks automatically reuse the stored chunk, optimizing storage capacity across repositories and namespaces.

---

## 2. Chunking and Manifest Hierarchy

```
+-----------------------------------------------------------+
|                     Artifact Payload                      |
|                  (Multi-Gigabyte Stream)                  |
+-----------------------------------------------------------+
                             │
            Chunking at 4 MiB Bounded Boundaries
                             ▼
  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
  │   Chunk 0    │     │   Chunk 1    │     │   Chunk N    │
  │   (4 MiB)    │     │   (4 MiB)    │     │   (<= 4 MiB) │
  │ Hash: BLAKE3 │     │ Hash: BLAKE3 │     │ Hash: BLAKE3 │
  └──────────────┘     └──────────────┘     └──────────────┘
         │                    │                    │
         └────────────────────┼────────────────────┘
                              ▼
           +---------------------------------------+
           |           ArtifactManifest            |
           | - manifest_id: BLAKE3(metadata)       |
           | - object_hash: BLAKE3(stream)         |
           | - total_size: u64                     |
           | - chunk_size: 4 MiB                   |
           | - chunks: [ChunkEntry, ...]           |
           +---------------------------------------+
```

### Manifest Schema (`ArtifactManifest`)
```json
{
  "manifest_id": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08",
  "object_hash": "5eb63bbbe01eeed093cb22bb8f5acdc3",
  "total_size": 8388608,
  "chunk_size": 4194304,
  "chunks": [
    { "index": 0, "offset": 0, "length": 4194304, "hash": "a1b2c3..." },
    { "index": 1, "offset": 4194304, "length": 4194304, "hash": "d4e5f6..." }
  ],
  "created_at": 1793798400000000000
}
```

---

## 3. Storage Hierarchy On Disk

The local storage node lays out artifacts into fan-out content-addressed directories:
```
<storage_root>/artifacts/
  ├── chunks/
  │    ├── a1/
  │    │    └── a1b2c3d4e5f6... (raw bytes)
  │    └── d4/
  │         └── d4e5f6789abc... (raw bytes)
  └── manifests/
       └── 9f/
            └── 9f86d081884c...json
```

Writing chunks is fully atomic: data is streamed to a temporary file (`.tmp_*`), flushed and synced to disk, and atomically renamed into place.

---

## 4. Distributed Cluster Protocol Extension

To maintain replication and durability across storage nodes, `spine-cluster` and `spine-storage-node` extend the native SPWP protocol with artifact storage commands:
1. `StoreChunk { chunk_hash: String, data: Vec<u8> }`
2. `GetChunk { chunk_hash: String } -> Option<Vec<u8>>`
3. `StoreManifest { manifest_id: String, manifest_json: String }`
4. `GetManifest { manifest_id: String } -> Option<String>`

`DistributedArtifactStore` orchestrates multi-node fan-out, enforcing configured durability policies (`SingleCopy`, `TwoCopies`, `ThreeCopies`) before acknowledging chunk or manifest writes to the caller.
