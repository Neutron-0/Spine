# Spine Forge: Batch 08 Integration Gap Register

This document tracks identified integration items, security boundaries, and runtime requirements surfaced during the transition from the foundational hosting engine (Batch 08) to the complete collaboration platform (Batch 09 and beyond).

---

## 1. Register of Identified Hosting & Integration Gaps

| Gap ID | Integration Item | Current State (Batch 08) | Target Milestone | Status / Mitigation |
|---|---|---|---|---|
| **GAP-01** | **Authenticated Native Push & Multi-Environment Fetch/Clone** | SPWP daemon supports Ed25519 and signed Capability Tickets (`ticket:<hex>.<sig>`). CLI resolves tickets via `POST /api/v1/repos/:owner/:repo/capability`. | **Batch 10 (COMPLETED)** | Fully resolved in Batch 10. Authoritative distributed write pipeline bridges CLI and Forge authentication seamlessly. |
| **GAP-02** | **Durability Acknowledgement on Verified Replicas** | Two-phase replication transactions (2PC) verify replica staging quorum before commit decisions. Storage dispatcher coordinates across live nodes. | **Batch 10 (COMPLETED)** | Fully resolved in Batch 10. `AuthoritativeRepositoryWriter` verifies quorum against durability policy (`SingleCopy`, `TwoCopies`, `ThreeCopies`). |
| **GAP-03** | **Runtime PostgreSQL Integration & Restart Durability** | SQLx migration scripts authored; `PgDatabase` supports connection pooling; tests run hermetically against `InMemoryDatabase`. | Batch 09 & 13 | Real migration runs tested with Postgres 16 container; schema validation for all 9 Batch 08 + 17 Batch 09 tables. |
| **GAP-04** | **TLS & Certificate Verification Across Distributed Boundaries** | TLS 1.3 implemented in `spine-net`; Forge API supports reverse-proxy TLS termination; Coordinator RPC currently uses TCP length-delimited JSON. | Batch 13 | Mutual TLS (mTLS) with internal cluster CA for Forge-to-coordinator and coordinator-to-storage-node RPC. |
| **GAP-05** | **Secure Cross-Platform Native CLI Credential Storage** | OS-native Windows Credential Manager (`advapi32.dll` / `wincred`) implemented with zero raw token exposure and restricted keyring fallback. | **Batch 10 (COMPLETED)** | Fully resolved in Batch 10. `OsCredentialManager` manages credentials with token masking redaction. |
| **GAP-06** | **Session, PAT, Password-Reset & Verification Lifecycle Coverage** | Unit and API tests verify token generation, hashing, and revocation; sliding expiry implemented. | Batch 09 | Covered in `auth_test.rs` and extended with organization invitation single-use token lifecycle tests. |
| **GAP-07** | **Provisioning Recovery on Abrupt Process Termination** | Outbox jobs persist idempotency key (`prov_<repo_id>`) with retry counter; worker updates status transactionally. | Batch 09 & 13 | Worker startup automatically claims orphaned `PROVISIONING` jobs older than heartbeat threshold. |
| **GAP-08** | **Private Repository & Entity Isolation Across Every Endpoint** | `PermissionService` enforces `404 Not Found` for unauthorized private repos to prevent metadata leakage. | Batch 09 | Extended to private organizations, internal teams, private issues, and pull request diffs. |
| **GAP-09** | **Full-Stack Docker Compose End-to-End Verification** | `docker-compose.yml` authored covering Postgres 16, Coordinator, 3 Storage Nodes, `spine-api`, and frontend. | Batch 13 (Deployment Gate) | Maintained and validated; independent Codex verification deferred until final deployment readiness gate. |

---

## 2. Deferred Capabilities Notice

- Independent Codex audit is deferred until the final deployment-readiness milestone.
- Cross-repository forks are deferred until native cross-repository object borrowing is implemented without compromising cluster boundaries.
- Continuous Integration (CI/CD) and build runners are planned for Batch 11.
