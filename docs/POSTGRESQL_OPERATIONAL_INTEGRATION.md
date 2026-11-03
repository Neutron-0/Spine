# PostgreSQL Operational Integration Specification

## 1. Relational Database Layer

Spine Forge pairs the content-addressed object store with an operational PostgreSQL relational database for structured metadata:
- **Users and Authentication**: Users, Argon2id password hashes, sessions, personal access tokens.
- **Access Control & Permissions**: Organizations, team memberships, repository collaborator assignments.
- **Collaboration Entities**: Issues, milestones, labels, pull requests, reviews, inline review comments.
- **Audit & Outbox**: Provisioning jobs outbox, activity event stream, audit trails.

## 2. Transaction Boundaries and Cross-Store Consistency

Because Git/Spine repository objects reside in distributed storage while issue/PR metadata resides in PostgreSQL, dual-write consistency is maintained via outbox and reconciliation patterns:

1. **VCS Write Precedes DB Mutation**:
   - The authoritative VCS write must commit first.
   - If the VCS write fails, the database transaction is never committed.
2. **Crash Reconciliation**:
   - If the server crashes after the VCS write succeeds but before the database transaction commits, the PR merge route performs automatic reconciliation:
     - On retry, it detects that the branch tip already points to a merge commit matching the PR's head and base commits.
     - It skips redundant object creation and advances the PR status to `MERGED`.
3. **Idempotency Keys**:
   - Repository provisioning and background operations use idempotency keys (`prov_<repo_id>`) to prevent duplicate initialization.
