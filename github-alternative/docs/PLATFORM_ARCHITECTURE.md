# SPINE FORGE PLATFORM ARCHITECTURE

## 1. System Overview
Spine Forge is a scalable, cloud-native developer collaboration platform designed to host Spine repositories, manage teams, track issues, review code, and coordinate distributed workflows.

```
+-------------------------------------------------------------------------------+
|                                  USER BROWSER                                 |
|                         React 18 + TypeScript + Vite                          |
+-------------------------------------------------------------------------------+
                                        | (HTTPS / WSS)
                                        v
+-------------------------------------------------------------------------------+
|                            EDGE REVERSE PROXY / TLS                           |
+-------------------------------------------------------------------------------+
                                        |
                   +--------------------+--------------------+
                   |                                         |
                   v                                         v
+--------------------------------------+  +------------------------------------+
|       Spine API (`spine-api`)        |  |     Spine Worker (`spine-worker`)  |
|  - Auth, Repositories, PRs, Issues   |  |  - Async jobs, Webhooks, Indexing  |
|  - Axum + Tokio (Rust)               |  |  - Redis/Postgres Job Queue        |
+--------------------------------------+  +------------------------------------+
        |                       |                            |
        | SQLx                  | gRPC / Internal RPC        |
        v                       v                            v
+------------------+  +--------------------------------------------------------+
| PostgreSQL 16+   |  |      Spine Storage Node Mesh (`spine-storage-node`)    |
| (Metadata Store) |  |  - Physical CAS repositories (.spine/)                 |
|                  |  |  - Raft placement & replica synchronization            |
+------------------+  +--------------------------------------------------------+
```

## 2. Core Service Components
1. **`spine-api`:** Front-facing Axum HTTP service handling JSON REST API endpoints, user sessions, permissions evaluation, and repository dispatching.
2. **`spine-storage-node`:** Autonomous storage microservice managing on-disk Spine repositories. Communicates with `spine-api` via authenticated RPCs, streaming object blobs and calculating diffs directly in Rust memory without spawning external child processes.
3. **`spine-worker`:** Asynchronous background processing engine for webhook delivery, search indexing, and repository statistics calculation.
4. **`spine-runner`:** Secure CI/CD agent executing workflow containers.
