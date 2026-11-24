# SPINE ARCHITECTURAL SPECIFICATION

Spine is an enterprise-grade, high-performance distributed version control ecosystem and modern developer collaboration platform engineered entirely in safe systems Rust.

This document describes the end-to-end architectural design, cryptographic storage engines, multi-tenant isolation boundaries, Git LFS data-flow pipelines, and disaster recovery guarantees of Spine.

---

## 1. System Ecosystem & Multi-Tier Topology

Spine separates concerns cleanly between the decentralized client-side version control engine (**Spine VCS**) and the centralized enterprise collaboration platform (**Spine Forge**).

```mermaid
flowchart TD
    subgraph ClientTier["Developer Workstation"]
        CLI["Spine CLI (spine)"]
        GitClient["Standard Git / Git LFS Client"]
        Browser["Web Browser (React + Vite SPA)"]
    end

    subgraph EdgeTier["Edge & Gateway Services"]
        ReverseProxy["Reverse Proxy (TLS Termination, Nginx / Traefik)"]
    end

    subgraph ForgeTier["Spine Forge Platform"]
        API["Axum HTTP / WebSocket API Server"]
        AuthModule["Auth & RBAC Module (Session, PAT, Capability Tokens)"]
        LfsHandler["Git LFS v2 Handler & Capability Verifier"]
        StorageEngine["Storage Abstraction Layer (Local CAS, S3 Provider)"]
        UploadRegistry["In-Flight Upload Registry (Durable Leases)"]
        GCWorker["Garbage Collection Worker (Tombstone & Grace Period)"]
    end

    subgraph DataTier["State & Persistence Tier"]
        PG[("PostgreSQL 16+ (Metadata, RBAC, lfs_objects)")]
        RedisCache[("Redis 7+ (Sessions & Rate Limiting)")]
        CAS[("Content-Addressed Storage (Chunks & Manifests)")]
    end

    CLI -->|"Native Spine Protocol"| API
    GitClient -->|"Git Smart HTTP & LFS Batch"| API
    Browser -->|"HTTPS / WebSocket"| ReverseProxy
    ReverseProxy --> API

    API --> AuthModule
    API --> LfsHandler
    LfsHandler --> UploadRegistry
    LfsHandler --> StorageEngine
    GCWorker --> UploadRegistry
    GCWorker --> StorageEngine

    API --> PG
    API --> RedisCache
    StorageEngine --> CAS
    GCWorker --> PG
```

---

## 2. Spine VCS Engine (`git-alternative/`)

Spine VCS operates as an independent, fully decentralized version-control engine with zero dependencies on C libraries (`libgit2`) or external Git subprocesses.

### 2.1 Content-Addressed Merkle DAG

Every artifact in Spine is stored as an immutable, content-addressed object identified by its cryptographic digest.

```mermaid
classDiagram
    class ObjectType {
        <<enumeration>>
        Blob = 0x01
        Tree = 0x02
        Commit = 0x03
        Tag = 0x04
    }

    class ObjectId {
        +[u8; 32] bytes
        +to_hex() String
        +from_hex(s) Result
    }

    class Blob {
        +ObjectId id
        +u64 size
        +Vec~u8~ raw_bytes
    }

    class TreeEntry {
        +u32 file_mode
        +String filename
        +ObjectId target_id
    }

    class Tree {
        +ObjectId id
        +Vec~TreeEntry~ entries
        +find_entry(path) Option
    }

    class Commit {
        +ObjectId id
        +ObjectId tree_id
        +Vec~ObjectId~ parent_ids
        +Signature author
        +Signature committer
        +String message
    }

    ObjectType <.. Blob
    ObjectType <.. Tree
    ObjectType <.. Commit
    Tree "1" *-- "many" TreeEntry : contains
    TreeEntry --> ObjectId : references
    Commit --> ObjectId : root tree and parents
```

#### Cryptographic Domain Separation
To prevent cross-type preimage or substitution attacks (e.g., crafting a blob that can parse as a valid commit or tree), Spine prepends an explicit domain separator before computing BLAKE3 digests:

$$\text{Digest} = \text{BLAKE3}\left(\text{"SPINE\0"} \mathbin{\Vert} \text{type\_byte} \mathbin{\Vert} \text{"\0"} \mathbin{\Vert} \text{payload\_len}_{\text{u64\_le}} \mathbin{\Vert} \text{"\0"} \mathbin{\Vert} \text{payload}\right)$$

### 2.2 On-Disk Repository Layout (`.spine/`)

```
.spine/
├── spine.toml             # Repository format version and configuration
├── HEAD                   # Active symbolic or direct reference pointer
├── index                  # Binary staging manifest with mtime/size caches
├── journal/               # Append-only transactional undo/redo log
├── objects/
│   ├── info/              # Multi-pack index (MIDX) descriptors
│   ├── [0-9a-f]{2}/       # Loose Zstandard-compressed objects
│   │   └── [0-9a-f]{62}   # Payload: Header (magic, type, size) + Zstd stream
│   └── pack/              # Compressed multi-object packfiles and indices (.spk, .spx)
└── refs/
    ├── heads/             # Local branch heads (refs/heads/main)
    └── tags/              # Cryptographic release tags
```

### 2.3 Transaction Journal & Undo/Redo Engine

Spine implements an append-only transaction journal (`.spine/journal/`) that tracks every state-modifying operation. Every command creates a reversible transaction record with forward and inverse actions, providing fearless, instant undo/redo capabilities.

```mermaid
stateDiagram-v2
    [*] --> CleanState : Initialize Repository

    CleanState --> StagedState : spine add <files>
    StagedState --> CleanState : spine undo (unstage files)

    StagedState --> CommittedState : spine commit -m "..."
    CommittedState --> StagedState : spine undo (revert commit to index)

    CommittedState --> BranchedState : spine branch create <name>
    BranchedState --> CommittedState : spine undo (delete created branch)

    CommittedState --> MergedState : spine merge <branch>
    MergedState --> CommittedState : spine merge --abort (restore pre-merge refs)

    CleanState --> [*] : Terminate
```

---

## 3. Spine Forge Platform (`github-alternative/`)

Spine Forge is the multi-tenant collaboration platform providing repository management, pull requests, issue tracking, and high-performance storage nodes.

### 3.1 Authentication & Authorization Matrix

Spine Forge supports three distinct authentication and authorization primitives:

```mermaid
flowchart LR
    Request["Incoming API Request"] --> AuthRouter{"Credential Type"}

    AuthRouter -->|"Cookie: spine_session"| SessionAuth["Session Authentication (Argon2id + Redis)"]
    AuthRouter -->|"Authorization: Bearer sp_pat_*"| PatAuth["Personal Access Token (SHA-256 Hash in PG)"]
    AuthRouter -->|"Authorization: Bearer sp_cap_*"| CapAuth["Scoped Capability Token (HMAC-SHA256)"]

    SessionAuth --> RbacCheck{"Repository Role"}
    PatAuth --> ScopeCheck{"Token Scopes"}
    ScopeCheck --> RbacCheck

    RbacCheck -->|"Admin"| AllowAll["All Operations (Settings, Collaborators, Push, Pull)"]
    RbacCheck -->|"Write"| AllowWrite["Code Push, LFS Upload, Branch Mutation"]
    RbacCheck -->|"Read"| AllowRead["Clone, Fetch, LFS Download, View Issues"]

    CapAuth --> CapVerify{"Verify Signature, Expiry, Repo, OID, and Operation"}
    CapVerify -->|"Valid"| AllowLfs["Permit Git LFS Transfer"]
    CapVerify -->|"Invalid or Mismatched"| Deny["403 Forbidden"]
```

---

## 4. Git LFS & Content-Addressable Storage (CAS) Pipeline

Git LFS in Spine uses Content-Addressable Storage (CAS) for high performance and global storage deduplication, while enforcing strict multi-tenant authorization.

### 4.1 LFS Data-Flow Architecture

```mermaid
sequenceDiagram
    autonumber
    actor Client as Git LFS Client
    participant API as Spine Forge LFS Route
    participant Lease as InFlightUploadRegistry
    participant CAS as Chunk CAS Engine
    participant DB as PostgreSQL (lfs_objects)

    Note over Client, DB: Step 1: Batch Negotiation
    Client->>API: POST /repos/{owner}/{repo}/info/lfs/objects/batch
    API->>DB: Query repository permission (Read/Write)
    DB-->>API: Authorized
    API->>DB: Check existing mapping in lfs_objects
    alt Object already exists in this repository
        API-->>Client: 200 OK (no actions needed, already present)
    else Object missing in this repository
        API-->>Client: 200 OK (upload URL with scoped capability token)
    end

    Note over Client, DB: Step 2: Streaming Upload & CAS Storage
    Client->>API: PUT /repos/{owner}/{repo}/info/lfs/storage/{oid} (Capability Token)
    API->>API: Validate capability token (op=upload, repo_id, oid, unexpired)
    API->>Lease: Register in-flight upload lease (oid, repo_id, pid)
    loop Stream chunks with backpressure
        Client->>API: Send body stream
        API->>CAS: Write CAS chunk and compute streaming SHA-256
        API->>Lease: Heartbeat lease renewal
    end
    API->>API: Verify final SHA-256 matches requested OID

    Note over Client, DB: Step 3: Verification & Durable Publication
    Client->>API: POST /repos/{owner}/{repo}/info/lfs/objects/verify (Capability Token)
    API->>API: Validate capability token (op=verify, repo_id, oid)
    API->>CAS: Confirm CAS manifest and chunks are durable on disk
    API->>DB: INSERT INTO lfs_objects (repository_id, oid, size) ON CONFLICT DO NOTHING
    API->>Lease: Release upload lease
    API-->>Client: 200 OK (Object Published)
```

### 4.2 Multi-Tenant Logical Isolation vs Shared Deduplication

```mermaid
flowchart TD
    subgraph RepoA["Tenant A: Repository 'alice/core'"]
        OIDA["LFS Object: sha256:aaa..."]
        MapA["lfs_objects mapping: repo_id=1, oid=aaa..."]
    end

    subgraph RepoB["Tenant B: Repository 'bob/fork'"]
        OIDB["LFS Object: sha256:aaa... (Same Content)"]
        MapB["lfs_objects mapping: repo_id=2, oid=aaa..."]
    end

    subgraph SharedCAS["Shared Content-Addressable Storage (CAS)"]
        CASBlob["Deduplicated Physical Chunk: sha256:aaa... (Stored Once)"]
    end

    subgraph TenantC["Tenant C: Repository 'charlie/private'"]
        NoAccess["No Mapping in lfs_objects"]
    end

    OIDA --> MapA --> CASBlob
    OIDB --> MapB --> CASBlob

    TenantC -.->|"Query Batch / Download"| Rejection["Denied (404/403) — Zero Existence Leak"]
```

---

## 5. Cross-Process & Multi-Worker GC Safety

To prevent Garbage Collection (GC) workers from pruning CAS chunks during long-running concurrent uploads, Spine enforces a multi-tier protection mechanism.

```mermaid
flowchart TD
    GCStart["GC Worker Starts Sweeping CAS"] --> ScanChunks["Enumerate Physical CAS Chunks"]

    ScanChunks --> CheckRef{"Referenced in lfs_objects (Any Repo)?"}
    CheckRef -->|"Yes"| RetainRef["Retain Chunk (In Active Use)"]

    CheckRef -->|"No"| CheckLease{"Active in InFlightUploadRegistry?"}
    CheckLease -->|"Yes (Heartbeat Valid)"| RetainLease["Retain Chunk (Upload In-Flight)"]

    CheckLease -->|"No"| CheckGrace{"Chunk mtime < Grace Period (e.g. 24h)?"}
    CheckGrace -->|"Yes (Young Orphan)"| RetainGrace["Retain Chunk (Safety Grace Period)"]

    CheckGrace -->|"No (Expired Orphan)"| Tombstone["Move to Tombstone Directory"]
    Tombstone --> AuditLog["Log Orphan Purge Event"]
    AuditLog --> FinalDelete["Delete Physical File from Disk"]
```

### In-Flight Upload Lease Lifecycle
- **Registration:** Created upon upload initiation (`.spine_uploads/<upload_id>.lease`).
- **Heartbeat:** Periodically updated while streaming bytes.
- **Expiration:** If an upload process crashes, leases expire after 15 minutes.
- **Cleanup:** GC treats expired leases without running PIDs as collectable orphans.

---

## 6. Disaster Recovery & Backup Architecture (SPNEBAK v2)

Spine Forge includes an authenticated backup and restore engine designed for enterprise recovery time objectives (RTO).

```mermaid
sequenceDiagram
    autonumber
    actor Admin as System Administrator / Cron
    participant Tool as spine-backup CLI
    participant API as Spine Forge API
    participant PG as PostgreSQL
    participant CAS as Physical Storage

    Admin->>Tool: spine-backup create --passphrase
    Tool->>API: POST /api/admin/maintenance/enter (Consistency Barrier)
    API-->>Tool: Maintenance Mode Active (Writes Paused)

    Tool->>PG: pg_dump (Atomic Metadata Snapshot)
    Tool->>CAS: Snapshot CAS Chunk Directory
    Tool->>Tool: Assemble SPNEBAK v2 Container
    Note over Tool: Encrypt with XChaCha20-Poly1305 / AES-256-GCM + Salt + Nonce
    Tool->>Tool: Compute AEAD Tag and Archive Digest
    Tool->>Admin: Emit encrypted archive (.spnebak)

    Tool->>API: POST /api/admin/maintenance/exit
    API-->>Tool: Maintenance Mode Released (Normal Traffic Resumed)
```

### SPNEBAK v2 File Container Format

```
+-------------------------------------------------------------------------+
| Magic Bytes: "SPNEBAK\0" (8 bytes)                                     |
+-------------------------------------------------------------------------+
| Format Version: 0x0002 (u16 big-endian)                                 |
+-------------------------------------------------------------------------+
| Cipher Suite: 0x01 (XChaCha20-Poly1305) or 0x02 (AES-256-GCM) (u8)     |
+-------------------------------------------------------------------------+
| Key Derivation Salt: Argon2id Salt (32 bytes)                           |
+-------------------------------------------------------------------------+
| Initialization Vector / Nonce (12 or 24 bytes)                          |
+-------------------------------------------------------------------------+
| Authenticated Associated Data (AAD): Manifest Header JSON               |
+-------------------------------------------------------------------------+
| Encrypted Payload: Compressed Metadata SQL + CAS Blobs                  |
+-------------------------------------------------------------------------+
| Poly1305 / GCM Authentication Tag (16 bytes)                            |
+-------------------------------------------------------------------------+
```

---

## 7. Migration & Upgrade Operability

Database schema evolutions are managed using versioned, forward-compatible SQL migrations with zero data loss.

### Legacy LFS Preflight & Remediation (`lfs_preflight`)
For existing installations upgrading to multi-tenant LFS isolation (Migration `0016_multitenant_lfs_mapping`):
- **Preflight Inspection:** The `lfs_preflight` utility audits existing `artifact_manifests` and `lfs_objects` rows.
- **Fail-Closed Validation:** Prevents migration if unmapped or ambiguous legacy rows exist without explicit operator assignment.
- **Operator Remediation:** Allows operators to supply an audited CSV mapping file to safely associate legacy manifests with their rightful owner repositories.

---

## 8. Summary of Security & Operational Invariants

1. **Deterministic Hashing:** BLAKE3 with domain separation prevents cross-type collisions.
2. **Path Confinement:** Working trees and storage roots strictly reject path traversal and symlink escapes.
3. **Multi-Tenant Logical Isolation:** Deduplicated CAS blobs are never accessible without an explicit repository mapping.
4. **Lease-Protected GC:** Active uploads cannot be garbage-collected, even across multiple worker processes.
5. **Crash Resilience:** Two-phase atomic tempfile renames prevent partially written objects or manifests upon system failure.
