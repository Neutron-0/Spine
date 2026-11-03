# Spine Forge: Architecture and System Specification

## 1. System Overview

Spine Forge is the native code-hosting, identity, and collaboration platform for the Spine version-control ecosystem. Unlike traditional developer platforms that treat Git as an opaque foreign sub-process, Spine Forge is deeply integrated into Spine's native storage engine, content-addressed object database, wire protocol, and distributed storage nodes.

```mermaid
flowchart TD
    CLI["Spine CLI (`spine auth`, `spine repo`)"]
    Web["Web UI (React + TypeScript)"]

    subgraph ControlPlane ["Spine Forge Control Plane"]
        API["Spine API Gateway (`spine-api`)"]
        AuthSvc["Auth & Identity Service"]
        PermSvc["Authorization Engine (RBAC + Isolation)"]
        RepoSvc["Repository Service"]
        Worker["Outbox Provisioning Worker"]
        DB[(PostgreSQL / InMemory DB)]
    end

    subgraph DistributedFabric ["Distributed Storage Fabric (Batch 07)"]
        Coord["Spine Coordinator (`spine-coordinator`)"]
        Store["Coordinator Store (Placements & Epochs)"]
        Node1["Storage Node 1"]
        Node2["Storage Node 2"]
        Node3["Storage Node 3"]
    end

    CLI -->|HTTP REST / Tokens| API
    Web -->|HTTP REST / Cookies| API
    API --> AuthSvc
    API --> PermSvc
    API --> RepoSvc
    RepoSvc --> DB
    AuthSvc --> DB
    PermSvc --> DB
    Worker --> DB
    Worker -->|Cluster RPC (TCP JSON)| Coord
    Coord --> Store
    Worker -->|Direct Placement Init| Node1
    Worker -->|Direct Placement Init| Node2
    Worker -->|Direct Placement Init| Node3
```

## 2. Component Architecture

### 2.1 Backend: `spine-api`
- **Framework**: Rust, Axum 0.7, Tokio async runtime.
- **Relational Storage**: Dual-backend support through `Database` trait:
  - `PgDatabase`: Production relational database via `sqlx` with connection pooling, migrations, and transactional execution.
  - `InMemoryDatabase`: Hermetic, fully concurrent in-memory store for unit tests and zero-external-dependency local development.
- **Storage VCS Service**: `NativeVcsService` directly inspects on-disk `.spine` repositories using `spine-format` and `spine-object-store`.
- **Outbox Pattern**: Asynchronous repository provisioning using transactional job queue in `repository_provisioning_jobs`.

### 2.2 Control-Plane Coordinator Integration
- When repositories are provisioned, `spine-api` communicates with `spine-coordinator` over the native TCP JSON RPC protocol.
- The coordinator assigns an authoritative placement plan based on rack-aware topology, capacity weights, and durability policy (`ThreeCopies`, `TwoCopies`, `SingleCopy`).
- The coordinator registers initial epoch 1 ownership and replica states, ensuring distributed consistency across independent storage nodes.

### 2.3 Native CLI Client
- The Spine CLI provides native `spine auth` (`login`, `status`, `logout`) and `spine repo` (`create`) commands.
- Zero external HTTP dependencies: implements a streamlined, robust async HTTP/1.1 client over `tokio::net::TcpStream`.
- Secure credential management stored at `~/.spine/credentials.toml` with token hashing and automatic expiration checks.

### 2.4 Web Frontend
- Single-page application built with React 18, TypeScript, and Vite.
- Responsive, dark-themed developer interface adhering to high accessibility and fast load times.
- Real-time branch switching, directory navigation, syntax-friendly file viewer, and commit graph browsing.
