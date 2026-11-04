# Spine Forge — Git Object Mapping Durability & Reconciliation

## 1. The Git-to-Spine Identity Challenge

Because Git calculates commit object identifiers (SHA-1) over formatted text buffers that include exact author and committer timestamps, timezones (e.g. `+0530`), and optional GPG signatures, reproducing the exact Git SHA-1 from a Spine commit model requires preserving the original raw Git object header and commit buffer.

Without exact SHA-1 preservation, independent Git clients cloning from Spine Forge would compute divergent commit hashes, causing `git pull` or `git fetch` to fail with `fatal: refusing to merge unrelated histories`.

---

## 2. On-Disk Persistence Model

Mappings and raw Git objects are scoped to each individual repository within `.spine/`:
```
.spine/
├── git_map.json            <-- Bidirectional GitOid <-> ObjectId mapping
├── git_objects/            <-- Cached raw Git commit and tree object buffers
│   └── 64e301d4...         <-- 1-byte type prefix + raw serialized Git payload
├── objects/                <-- Native Spine content-addressed object store (BLAKE3)
└── refs/                   <-- Native Spine branch and tag references
```

### Crash-Consistent Atomic Writing:
In Batch 17, `save_git_map` was hardened:
```rust
fn save_git_map(spine_dir: &Path, git_to_spine: &HashMap<GitOid, ObjectId>) {
    let map_file = spine_dir.join("git_map.json");
    let mut map = HashMap::new();
    for (goid, soid) in git_to_spine {
        map.insert(goid.to_hex(), soid.to_hex());
    }
    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let tmp_file = map_file.with_extension("tmp");
        if fs::write(&tmp_file, json).is_ok() {
            let _ = fs::rename(&tmp_file, &map_file);
        }
    }
}
```
Writing through `.tmp` followed by atomic filesystem rename guarantees that half-written or torn files cannot occur if the process terminates abruptly.

---

## 3. Dynamic Reconciliation Engine

For commits created natively within Spine (e.g. via Spine CLI, web UI editor, or pull request merge) where no Git client originally pushed raw buffers, the server invokes `reconcile_git_map`:

```rust
pub fn reconcile_git_map(
    spine_dir: &Path,
) -> Result<(HashMap<GitOid, ObjectId>, HashMap<ObjectId, GitOid>), GitTransportError>
```

1. Scans `.spine/refs/heads/` and `.spine/refs/tags/`.
2. Inspects `git_map.json`.
3. For any Spine commit without a recorded Git OID, synthesizes canonical Git tree and commit objects using `GitExporter::export_commit_recursive_pub`.
4. Updates the bidirectional map and atomically saves `git_map.json`.
5. Guarantees Git clients can fetch, clone, and inspect native commits without error.
