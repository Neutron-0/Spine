# Spine Forge — Native Git LFS HTTP Architecture

## 1. System Overview

Spine Forge provides native, standard Git LFS (Large File Storage) HTTP transport compatibility, enabling official Git LFS clients (`git-lfs` v3.7+) to push, fetch, and pull large binary assets directly against Spine Forge repositories without external wrappers or proxy daemons.

The implementation builds upon Spine's native distributed content-addressed storage engine (`spine-artifact`) and authoritative repository write pipeline, maintaining strict repository isolation, quota enforcement, and pre-receive integrity verification.

```mermaid
sequenceDiagram
    autonumber
    actor Dev as Developer (git + git-lfs)
    participant Forge as Spine Forge API (Axum)
    participant Auth as Auth & Capability Signer
    participant Store as spine-artifact Store (BLAKE3)
    participant VCS as Native Spine VCS (.spine/)

    Note over Dev,Forge: Phase 1: LFS Object Upload
    Dev->>Forge: POST /{owner}/{repo}.git/info/lfs/objects/batch (Upload, SHA-256 OIDs)
    Forge->>Auth: Validate PAT/Credentials & Write Permission
    Forge->>Store: Query Existing Manifests (Deduplication Check)
    Forge->>Auth: Generate Scoped HMAC Transfer Tokens
    Forge-->>Dev: 200 OK (Upload & Verify Actions with Scoped Tokens)

    Dev->>Forge: PUT /{owner}/{repo}.git/info/lfs/objects/{oid} (Streamed Payload)
    Forge->>Auth: Verify Scoped Token (Repo + OID + Op + Expiry)
    Forge->>Store: Stream into Chunks (4 MiB) & Save Manifest
    Forge-->>Dev: 200 OK

    Dev->>Forge: POST /{owner}/{repo}.git/info/lfs/verify (Confirm Persisted Size)
    Forge->>Store: Verify Manifest Exists & Matches Declared Size
    Forge-->>Dev: 200 OK

    Note over Dev,Forge: Phase 2: Git Smart HTTP Receive-Pack
    Dev->>Forge: POST /{owner}/{repo}.git/git-receive-pack (Commit + Pointer Blobs)
    Forge->>Store: Pre-Receive Hook: Verify Referenced LFS Objects Exist in Store
    Forge->>VCS: Authoritative CAS Ref Update
    Forge-->>Dev: 200 OK (report-status ok)
```

---

## 2. Core Architectural Principles

1. **Zero External Runtime Dependencies:**
   - Pure Rust implementation using `axum`, `tokio`, `sha2`, `spine-artifact`, and `spine-migration`.
   - Neither Git, Git LFS, nor `libgit2` are invoked by the running backend server.
2. **Standard Git LFS Protocol Compliance:**
   - Implements the official Git LFS Batch API v1 and Basic Transfer Adapter.
   - Emits standard `application/vnd.git-lfs+json` media types.
3. **Internal Storage Integration (`spine-artifact`):**
   - High-performance, deduplicated chunk storage using FastCDC/stream chunking and BLAKE3 verification.
   - Bidirectional identity mapping between external Git LFS SHA-256 OIDs and internal BLAKE3 manifests.
4. **Authoritative Consistency & Pre-Receive Fencing:**
   - `git-receive-pack` parses incoming Git commits and inspects all new blobs.
   - If an LFS pointer is introduced without its backing binary object having been uploaded and verified in `spine-artifact`, the push is rejected immediately on the server side with a descriptive client-visible error.
5. **Short-Lived Scoped Capabilities:**
   - Upload and download URLs are protected by HMAC-SHA256 scoped capability tokens bound to repository ID, object SHA-256 OID, operation, max allowed size, and TTL.
