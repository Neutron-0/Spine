# SPINE DISTRIBUTED STORAGE ARCHITECTURE

## 1. Decentralized Storage Topology
Spine Forge is engineered to scale across heterogeneous, independent storage nodes without single-point bottlenecks:

```
                  +--------------------------------+
                  |         PostgreSQL DB          |
                  |  - Repository routing table    |
                  |  - Primary & Replica node IDs  |
                  +--------------------------------+
                                  |
                +-----------------+-----------------+
                |                                   |
                v                                   v
+-------------------------------+   +-------------------------------+
|      Storage Node A           |   |      Storage Node B           |
|  - Node ID: `node-eu-01`      |   |  - Node ID: `node-us-01`      |
|  - Capacity: 10 TB            |   |  - Capacity: 20 TB            |
|  - Active Repos: 4,500        |   |  - Active Repos: 9,200        |
|  - Local CAS (`.spine/`)      |   |  - Local CAS (`.spine/`)      |
+-------------------------------+   +-------------------------------+
                ^                                   ^
                +==== Cryptographic Sync Stream ====^
```

## 2. Core Architectural Guarantees
1. **Decoupled Metadata & Content:**
   - The central PostgreSQL database tracks node memberships, cluster health, capacity statistics, and repository placement maps (`repository_id` $\rightarrow$ `[primary_node_id, replica_1_id, replica_2_id]`).
   - The relational database **never** stores repository blobs, trees, or packfiles.
2. **Repository Portability:**
   - Because a Spine repository is entirely content-addressed and self-describing (`.spine/`), moving a repository between storage nodes simply involves streaming missing object hashes. Canonical URLs and IDs never change.
3. **Signed Replication Streams:**
   - Storage nodes replicate repository changes using mutual TLS and Ed25519-signed replication envelopes.
4. **Failure Detection & Quorum:**
   - Heartbeats monitor node liveliness. In the event of primary node failure, an elected replica promotes itself and updates the routing table.
