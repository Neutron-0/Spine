# Spine Master Roadmap & Architectural Progression

## Completed Milestones

### Batch 01: Foundation, Object Store & First Working Engine
- Native `.spine/` repository format and BLAKE3 / SHA-256 content-addressing.
- Zstandard object compression for blobs, trees, and commits.
- Binary staging index and basic CLI (`init`, `config`, `add`, `status`, `commit`, `log`, `show`, `fsck`).

### Batch 02: Branching, Switching, Merging & Journaling
- Full branch management and safe working-tree switching (`switch`, `branch`, `diff`).
- 3-way tree merge with conflict detection, marker generation, and resolution.
- Multi-parent commit support.
- Two-phase transaction operation journal (`undo`, `redo`).

### Batch 03: Storage Engine V2
- Native `.spk` packfiles and `.spidx` fanout binary indexes.
- Multi-pack catalog (`packs.catalog`) and composite object store.
- Object repack and conservative garbage collection (`spine repack`, `spine gc`).

### Batch 04: Native Wire Protocol & Networking
- Spine Wire Protocol (SPWP) v1 with length-delimited framing and BLAKE3 checksums.
- `spine-daemon` server runtime.
- Remote repository operations (`clone`, `fetch`, `push`, `pull`, `remote`).

### Batch 05: Cryptographic Identity & Transport Security
- Ed25519 asymmetric peer identity and cryptographic challenges.
- TLS 1.3 transport security over raw TCP.
- Replay attack defenses and lease-based atomic force-push (`--force-with-lease`).
- Object-closure graph verification before push acceptance.

### Batch 06: Advanced Local VCS, History Manipulation & Multi-Worktree Engine
- History manipulation: `reset` (soft, mixed, hard), `restore`, `revert`, `cherry-pick`.
- Linear commit replay engine: `rebase` (`--onto`, `--continue`, `--skip`, `--abort`).
- Stack-based stash management: `stash` (`push`, `list`, `show`, `apply`, `pop`, `drop`, `clear`).
- Linked multi-worktree engine: `worktree` (`add`, `list`, `remove`, `prune`) with shared pack storage and branch ownership locks.
- Annotated cryptographic tags: `tag` (`ObjectType::Tag = 0x04`).
- Inspection & Localization: `blame` (line provenance) and `bisect` (binary search defect isolation).
- Workspace filesystem helpers: `mv`, `rm`, `clean`.
- GC protection across worktrees, stashes, tags, and in-flight states.

---

## Next Milestones

### Batch 07: Large Object Management & Partial Worktrees
- Spine LFS: Content-addressed external asset store with chunked streaming and deduplication.
- Sparse checkouts and path-filtered indexes for monorepo scale.

### Batch 08: Spine Forge Architecture & Rust Backend
- Forge daemon with multi-tenant repository hosting.
- Role-based access control, organizational permissions, and SSH/SPWP gateway.
- Pull request engine with automated mergeability testing and review commenting.

### Batch 09: Spine Forge Web Frontend
- High-performance TypeScript / WebAssembly repository browser, code viewer, and graph visualizer.
- Real-time review and collaborative workflow interface.
