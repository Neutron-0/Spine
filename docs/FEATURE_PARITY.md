# SPINE FORGE FEATURE PARITY & PLATFORM ROADMAP

This document tracks feature parity between Spine Forge and major platforms like GitHub / GitLab / Gitea.

## Parity Classification
- **Phase 1 (Current / Batch 01):** Core VCS Engine, Staging, Snapshots, History, Integrity, CLI, and Platform Scaffolding.
- **Phase 2 (Beta Target):** Repositories, Authentication, Web UI, Tree Browser, Issues, Pull Requests.
- **Phase 3 (General Availability):** Organizations, Protected Branches, Code Reviews, Webhooks, Package Registry.
- **Phase 4 (Enterprise / Ecosystem):** Ephemeral CI/CD Runners, Distributed Raft Object Replication, Secret Scanning.

---

## Comprehensive Feature Inventory

| Area | Feature | Classification | Status | Notes |
|---|---|---|---|---|
| **VCS Engine** | Content-Addressed Storage | Phase 1 | **Implemented** | BLAKE3 + Zstandard CAS |
| **VCS Engine** | Staging & Indexing | Phase 1 | **Implemented** | Stat-cached binary manifest |
| **VCS Engine** | Snapshots & Commits | Phase 1 | **Implemented** | Immutable Merkle DAG |
| **VCS Engine** | History Traversal & Log | Phase 1 | **Implemented** | `spine log` / `spine show` |
| **VCS Engine** | Integrity Verification | Phase 1 | **Implemented** | Full `spine fsck` verification |
| **VCS Engine** | Branch & Switch | Phase 2 | Planned | Local branch pointers |
| **VCS Engine** | 3-Way Merge & Conflicts | Phase 2 | Planned | Structural conflict nodes |
| **VCS Engine** | Compaction Packfiles | Phase 3 | Planned | Delta-compressed packs |
| **Platform** | User Accounts & Profiles | Phase 2 | Planned | Argon2id + Sessions |
| **Platform** | Public & Private Repos | Phase 2 | Planned | RBAC permission matrix |
| **Platform** | Web Repository Browser | Phase 2 | Underway | React tree viewer component |
| **Platform** | Web Unified Diff Viewer | Phase 2 | Underway | Line diff component |
| **Platform** | Issues & Milestones | Phase 2 | Planned | Markdown task tracker |
| **Platform** | Pull Requests | Phase 2 | Planned | Branch comparison & merge |
| **Platform** | Code Reviews & Inline Notes| Phase 3 | Planned | Threaded diff comments |
| **Platform** | Protected Branches | Phase 3 | Planned | Rule enforcement engine |
| **Platform** | Organizations & Teams | Phase 3 | Planned | Multi-level tenant groups |
| **Platform** | Webhooks & Events | Phase 3 | Planned | Event bus + HMAC signatures |
| **Platform** | Releases & Assets | Phase 3 | Planned | Tag-anchored asset downloads |
| **Platform** | Large-File Storage (LFS) | Phase 3 | Planned | Native CAS chunk streaming |
| **Platform** | Package Registries | Phase 4 | Planned | Cargo / npm / OCI containers |
| **Platform** | CI/CD Runner Engine | Phase 4 | Planned | Ephemeral isolated containers|
| **Platform** | Distributed Storage Nodes | Phase 4 | Architecture Ready | Decentralized node federation|
| **Platform** | Audit Logs | Phase 4 | Planned | Immutable audit trail |
