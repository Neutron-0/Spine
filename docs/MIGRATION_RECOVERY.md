# MIGRATION FAULT TOLERANCE, CHECKPOINTS, AND RECOVERY

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. RECOVERY & IDEMPOTENCE GOALS

Importing large multi-gigabyte repositories with hundreds of thousands of commits can take minutes or hours. Networks fail, processes are interrupted, and machines restart.

The Spine Migration Engine is designed with:
1. **Zero Data Corruption:** Partial runs leave no orphaned or corrupted repository state.
2. **Resumability:** Interrupted migrations resume from the last completed stage without re-converting already imported objects.
3. **Idempotence:** Re-running a completed migration produces identical results without duplicate objects or dangling references.

---

## 2. CHECKPOINT MANIFEST ARCHITECTURE

Every migration execution maintains a structured JSON manifest located at:
`<migration_dir>/.spine-migration-manifest.json`

```rust
pub struct MigrationManifest {
    pub migration_id: String,
    pub source_kind: MigrationSourceKind,
    pub source_path: PathBuf,
    pub target_repo_path: PathBuf,
    pub current_phase: MigrationPhase,
    pub completed_phases: Vec<MigrationPhase>,
    pub stats: MigrationStats,
    pub last_updated_epoch_secs: u64,
}
```

### Execution Phases

```
[Inspect] -> [Plan] -> [Objects Ingest] -> [Trees Converted] -> [Commits Mapped] -> [Refs Written] -> [Checkout] -> [Verified]
```

At the completion of each phase, the engine:
1. Flushes in-flight object stores to disk.
2. Atomically writes a temporary `.manifest.json.tmp` file.
3. Atomically renames the temporary file over `.spine-migration-manifest.json`.

---

## 3. RESUMING AN INTERRUPTED MIGRATION

If an import fails or is cancelled mid-way:

```bash
# Check current migration progress:
spine migrate report --target /path/to/spine-repo

# Resume execution from last checkpoint:
spine migrate resume --target /path/to/spine-repo
```

### Recovery Logic:
- If `Objects Ingest` completed: The engine skips loose and packfile object parsing, retaining the generated object mapping table.
- If `Commits Mapped` completed: The engine bypasses commit topological traversal and proceeds directly to reference resolution and working tree checkout.
- If an unrecoverable failure occurred: The rollback handler restores the previous `.spine/refs/` state or purges newly staged objects.

---

## 4. INTEGRITY VERIFICATION POST-RECOVERY

Upon resumption or completion, the engine runs an internal verification pass:
- Verifies object closure: Checks that all commits point to valid root trees, and that all tree entries resolve to existing blobs or subtrees in the CAS.
- Validates that `HEAD` resolves to a valid branch reference.
- Confirms working tree clean state.
