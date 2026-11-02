# ADR-0008: Distributed Storage Fabric Architecture

## Status
Accepted

## Context
Spine requires a resilient distributed storage infrastructure to support multi-node hosting, high availability, and replica self-healing for the upcoming Spine Forge platform. 

A naive file-sharing approach (such as mounting NFS/SMB or copying loose directories) suffers from split-brain scenarios, silent bitrot, race conditions during concurrent pushes, and uncoordinated deletions. Spine requires an explicit separation of the control plane and data plane:
1. **Control Plane (`spine-coordinator`)**: Authoritative owner of cluster metadata, node registration, repository placements, replication policies, ownership epochs, and transaction commit decisions.
2. **Data Plane (`spine-storage-node`)**: Independently running storage daemon processes storing verified repository packfiles, catalogs, and loose objects, and serving replication transfers.

## Decision

1. **Topological Model**:
   - The cluster consists of one authoritative coordinator process and $N$ independent storage node processes (minimum 3 for high-durability topologies).
   - Storage nodes run as separate daemon processes with independent disk directories, listening on distinct network ports, authenticating via mutual Ed25519 cryptographic identities.
   - Failure domains (host, rack, zone, region) are explicitly declared during enrollment and incorporated into placement decisions.

2. **Full Repository Placement**:
   - Each repository is assigned to an authoritative primary storage node and one or more full replica storage nodes according to a configured `DurabilityPolicy` (`SINGLE_COPY`, `TWO_COPIES`, `THREE_COPIES`).
   - Repositories are fully replicated on each assigned node (no arbitrary object splitting across unrelated nodes), ensuring any fully synchronized replica can be promoted to primary during failover without object reassembly.

3. **Storage Engine Integration**:
   - Storage nodes reuse Spine's native storage format: loose objects, `.spk` packfiles, `.spidx` 256-bucket fanout indexes, and `packs.catalog`.
   - Every received packfile or object is cryptographically verified (BLAKE3/SHA-256 checksums and object graph closure) prior to durable acknowledgement.

4. **Persistence Abstraction**:
   - The coordinator defines a `CoordinatorMetadataStore` abstraction. Production deployments target PostgreSQL with versioned migrations (`migrations/`), while local and testing environments utilize an embedded durable store for deterministic, self-contained test execution.

## Consequences
- Clean separation between policy/metadata (coordinator) and object byte storage (storage nodes).
- Independent deployability of storage nodes.
- High resilience: single node failures never compromise repository availability if eligible replicas exist.
