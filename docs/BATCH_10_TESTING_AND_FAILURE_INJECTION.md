# Batch 10 Testing and Fault-Injection Verification

## 1. Test Suite Coverage

Batch 10 introduces focused automated test suites covering all authoritative write, security, and networking operations:

### A. VCS / Distributed Engine Tests (`git-alternative/apps/spine-cli/tests`):
1. `authoritative_write_pipeline_test.rs`:
   - Single-copy durability policy verification.
   - CAS mismatch and stale reference rejection.
   - Branch protection enforcement: direct push rejection when PR is required.
   - Review approval threshold enforcement: rejection on insufficient approvals.
   - Change request rejection.
2. `remote_e2e_hosting_test.rs`:
   - Multi-process end-to-end integration: SPWP daemon listener, client clone with contributor capability ticket, file addition and push, verification by secondary client clone, and reader-role push rejection.
3. Total CLI / Engine Tests: **109 tests passing**.

### B. Forge API Backend Tests (`github-alternative/backend/apps/spine-api/tests`):
1. `capability_test.rs`:
   - BLAKE3 keyed hash signing and verification of `CapabilityTicket`.
   - Tampered signature rejection.
   - Expired timestamp rejection.
   - Key mismatch rejection.
   - Route `POST /api/v1/repos/:owner/:repo/capability` authorization.
   - Private repository isolation: unauthorized requests return `404 Not Found`.
2. `authoritative_merge_test.rs`:
   - Pull request 3-way merge producing `.spk` and `.spidx` pack files.
   - Execution through `AuthoritativeRepositoryWriter`.
   - Durability status verification (`/status/durability`).
   - Rejection when `CHANGES_REQUESTED` exists.
3. Total Backend Tests: **25 tests passing**.

### C. Frontend Verification:
- `spine-forge-frontend`: 0 TypeScript compiler errors, clean Vite production bundle.

## 2. Fault Injection and Boundary Scenarios

| Scenario | Injected Condition | Expected Behavior | Verification Status |
| :--- | :--- | :--- | :--- |
| **CAS Race Condition** | Reference tip moved between negotiation and commit | Immediate abort with `StaleReference` error | Verified |
| **Epoch Fencing** | Storage node presented with outdated epoch | Immediate rejection with `OwnershipEpochMismatch` | Verified |
| **Durability Quorum Failure** | Insufficient replicas acknowledge Phase 1 staging | 2PC aborts, staged directories cleaned up | Verified |
| **Missing Object Injection** | Commit references an un-staged blob | Object closure validation detects gap and rejects | Verified |
| **Unauthorized Private Repo Discovery** | Attacker queries private repository without token | Returns `404 Not Found` (zero information leak) | Verified |
| **Reader Pushing to Protected Branch** | Read-only capability ticket attempts push | Rejected with permission denied before staging | Verified |
