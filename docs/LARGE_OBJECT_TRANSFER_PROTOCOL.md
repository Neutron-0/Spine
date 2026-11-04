# LARGE OBJECT TRANSFER PROTOCOL

## 1. Overview
The Large Object Transfer Protocol provides reliable, resumable, parallel multi-gigabyte upload and download over HTTP and native SPWP streams.

---

## 2. Resumable Upload Lifecycle

```
Client                                                  Spine API / Storage Node
  │                                                                 │
  │ 1. POST /api/v1/repos/:o/:n/lfs/objects/batch                   │
  │    (Object manifest metadata, chunk hashes, total bytes)        │
  ├────────────────────────────────────────────────────────────────►│
  │                                                                 │
  │ 2. Return Session ID + Missing Chunks List                      │
  │    { session_id: "...", missing_chunks: [0, 2, 3] }             │
  │◄────────────────────────────────────────────────────────────────┤
  │                                                                 │
  │ 3. Parallel PUT Chunks                                          │
  │    PUT /lfs/sessions/:id/chunks/:index/:hash                    │
  ├────────────────────────────────────────────────────────────────►│
  │    (Bounded 4 MiB streaming body)                               │
  │◄────────────────────────────────────────────────────────────────┤
  │    HTTP 200 OK                                                  │
  │                                                                 │
  │ 4. POST /lfs/sessions/:id/finalize                              │
  ├────────────────────────────────────────────────────────────────►│
  │                                                                 │
  │ 5. Assemble Manifest, Record Quota, Durability Replicate        │
  │◄────────────────────────────────────────────────────────────────┤
  │    HTTP 200 OK (ArtifactManifest Created)                       │
```

---

## 3. Session State Machine

A transfer session progresses through explicit states:

```
    [Created]
        │
        ├── (All chunks already present on server)
        ├────────────────────────────┐
        │                            ▼
        ▼                      [Finalizing]
   [Uploading]                       │
   (Iterative chunk uploads)         │
        │                            │
        ▼                            ▼
   [Finalizing] ──────────────► [Completed]
        │                            │
        ▼                            ▼
    [Failed]                     (Immutable)
```

- **Created**: Session initiated with expected chunk count, total bytes, and whole-object hash. Quota is pre-reserved.
- **Uploading**: Client uploads missing chunks concurrently. Each chunk is verified against its declared BLAKE3 hash.
- **Finalizing**: Client requests completion. The server validates that 100% of declared chunks are physically persisted and intact.
- **Completed**: Manifest is published to repository/release/package record. Session marked finished.
- **TTL Expiration**: Uncompleted sessions expire after 24 hours. Orphaned chunks without manifest references are reclaimed during next GC cycle.
