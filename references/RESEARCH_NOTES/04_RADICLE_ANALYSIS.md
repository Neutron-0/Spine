# RESEARCH NOTE 04: RADICLE HEARTWOOD P2P & DECENTRALIZED REPLICATION

## 1. Core Model & Philosophy
Radicle Heartwood is a sovereign peer-to-peer code collaboration network built on Git:
- **Decentralized Identities:** Repositories, projects, and users are identified by cryptographic keys (Ed25519 node IDs and public DID documents).
- **Signed References:** Git refs are signed by repository maintainers (`rad/sigrefs`) to guarantee authenticity without central authority.
- **Collaborative Objects (COBs):** Issues, patches, and discussions are stored as CRDT-like content-addressed object streams inside the repository itself.
- **P2P Node Mesh:** Heartwood nodes gossip repository state, discover peers, and sync refs over an authenticated Noise protocol wire format.

## 2. Key Subsystems Inspected
- `radicle/src/storage/git.rs`: Multi-tenant repository storage organizing user-specific ref namespaces (`refs/namespaces/<remote_id>/refs/heads/...`).
- `radicle-node/src/service.rs`: Event-driven state machine coordinating peer connections, gossip routing, and fetch scheduling.

## 3. What Spine Adopts
1. **Decentralized Storage Node Federation:** For Spine Forge, repositories can replicate across autonomous storage nodes using cryptographically signed manifests.
2. **Namespace Isolation:** Multi-node synchronization organizes replica branches by node identity, preventing race conditions.

## 4. What Spine Avoids
1. **Embedding Collaboration State in Raw Git Refs:** Heartwood's COBs incur high Git ref traversal overhead. Spine Forge uses relational databases (PostgreSQL) with event-sourcing for issues/PRs while archiving signed snapshots to the CAS for auditability.
