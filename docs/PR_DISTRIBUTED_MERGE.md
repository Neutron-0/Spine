# Pull Request Distributed Merge Pipeline

## 1. Unified Merge Pipeline

In Spine Forge, merging a pull request does not use local temporary Git clones or separate internal write hacks. Merging is fully integrated into the distributed storage pipeline:

1. **Merge Computation**:
   - The tree objects of base and head commits are merged using 3-way text and tree merge algorithms (`spine_merge::merge_3way_trees`).
   - If conflicts exist, the API returns a structured report listing conflicting files and paths.
2. **Object Packaging**:
   - If clean, the new tree and merge commit objects (with two parents: `[base_oid, head_oid]`) are written to the primary object store.
   - All newly created objects are packaged into a genuine binary `.spk` packfile using `spine_net::pack_transfer::create_pack_file`.
   - A `.spidx` pack index is constructed using `spine_object_store::pack_index::PackIndexWriter`.
3. **Authoritative Submission**:
   - The merge is submitted to `AuthoritativeRepositoryWriter::execute_write`.
   - `PrProtectionContext` is passed with review counts, change request status, and comment resolution state.
   - Durability quorum is achieved across the cluster (Phase 1 2PC).
   - Atomic ref update is finalized on primary and replicas (Phase 2 2PC).
4. **Metadata and Issue Closing**:
   - The pull request state updates to `MERGED` in PostgreSQL / database.
   - Commit messages are scanned for keywords (`fixes #123`, `closes #456`) and matching issues are automatically transitioned to `CLOSED`.
   - Activity events and user notifications are dispatched.
