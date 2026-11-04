# GIT OBJECT MAPPING & TOPOLOGY TRANSLATION

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. HASHING PARADIGMS & IDENTIFIER TRANSLATION

Git and Spine use distinct cryptographic hashing schemes and framing headers:

| Property | Git Format | Spine Native Format |
|---|---|---|
| **Primary Hash Algorithm** | SHA-1 (legacy) or SHA-256 (modern) | BLAKE3 (256-bit) |
| **Object Framing Header** | `<type_name> <content_len>\0<payload>` | `SPINE\0<type_byte><payload_len_u64_le><payload>` |
| **Identifier Representation** | 20 bytes (SHA-1) or 32 bytes (SHA-256) | 32 bytes (BLAKE3) |
| **Format Compression** | Deflate / zlib | Zstandard (zstd level 3) |

Because framing headers and algorithms differ, **a Git commit and a Spine commit representing identical trees and metadata will produce completely different cryptographic hashes**.

To preserve full structural fidelity during import and export without altering commit history, `spine-migration` maintains an in-memory and manifest-persisted bidirectional translation map:

```rust
pub struct ObjectMappingRegistry {
    git_to_spine: HashMap<GitOid, ObjectId>,
    spine_to_git: HashMap<ObjectId, GitOid>,
}
```

---

## 2. STRUCTURAL OBJECT TRANSLATION

### 2.1 Blobs (Files)

- **Git Blob:** Raw byte sequence prefixed by `blob <len>\0`.
- **Spine Blob:** Raw byte sequence stored as `LooseObject::Blob(bytes)`.
- **Mapping:** 1-to-1 byte identity. The binary file content is identical.

### 2.2 Trees (Directories)

#### Mode Mapping

| Mode | Git Octal String | Spine FileMode | Description |
|---|---|---|---|
| Regular File | `100644` | `FileMode::Regular` | Standard non-executable file |
| Executable File | `100755` | `FileMode::Executable` | Executable script/binary |
| Symbolic Link | `120000` | `FileMode::Symlink` | Posix symlink |
| Directory (Tree) | `040000` | `FileMode::Directory` | Child directory / subtree |
| Submodule (Commit)| `160000` | `FileMode::Submodule` | Git submodule commit pointer |

#### The Canonical Git Tree Sorting Invariant

A critical interoperability requirement of Git is that **tree entries are sorted by raw byte comparison, except directories are compared as if their entry name ended with a trailing slash (`/`)**.

```rust
pub fn compare_git_tree_entries(a: &GitTreeEntry, b: &GitTreeEntry) -> std::cmp::Ordering {
    let mut name_a = a.name.as_bytes().to_vec();
    if a.is_directory() {
        name_a.push(b'/');
    }
    let mut name_b = b.name.as_bytes().to_vec();
    if b.is_directory() {
        name_b.push(b'/');
    }
    name_a.cmp(&name_b)
}
```

*Consequence of Violation:* Failing to append `/` when comparing directories will cause Git to report `tree not sorted` during `git fsck --full` and corrupt tree traversal in native Git tools. `spine-migration` enforces this ordering strictly.

### 2.3 Commits

A Git commit is structured as text fields followed by an empty line and message:
```text
tree <40-hex-oid>
parent <40-hex-oid>
author Alice <alice@example.com> 1700000000 +0000
committer Bob <bob@example.com> 1700000000 +0000

Initial commit message
```

- **Parents:** Translated from `GitOid` to `ObjectId` using the topological commit mapping table.
- **Tree:** Translated from root Git tree `GitOid` to root Spine tree `ObjectId`.
- **Identity & Timestamp:** Author and committer name, email, Unix timestamp in seconds, and UTC timezone offset are preserved verbatim.
- **Message:** Commit message string preserved byte-for-byte.

### 2.4 Annotated Tags

Git annotated tags wrap a target object with tagger identity, date, and message:
```text
object <40-hex-oid>
type commit
tag v1.0.0
tagger Jane Doe <jane@example.com> 1700000000 +0000

Release version 1.0.0
```

Spine's `spine_format::AnnotatedTag` stores `target_id`, `target_kind`, `name`, `tagger`, and `message`. Tag mapping translates the target object ID and preserves tagger metadata and signatures.

---

## 3. DETERMINISTIC ROUNDTRIP VERIFICATION

During roundtrip export (`Spine -> Git -> Spine`), the following invariants hold:
1. Every tree generates the exact same directory layout and file modes.
2. File blobs maintain identical byte content and checksums.
3. Commit graphs maintain identical ancestor relationships (parents, merges, octopus merges).
4. Commits retain identical author, committer, and timestamps.
5. All references (`refs/heads/*`, `refs/tags/*`) resolve to corresponding converted commits.
