# MIGRATION FORMAT SPECIFICATION & TECHNICAL REFERENCE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. SUPPORTED INPUT AND OUTPUT FORMATS

| Format Category | Specific Format / Extension | Import Support | Export Support | Engine Module |
|---|---|---|---|---|
| **Git Working Tree** | `.git/` directory layout | Full (loose & pack) | Full | `git_importer.rs`, `git_exporter.rs` |
| **Bare Git Repository** | `repo.git/` layout | Full (loose & pack) | Full | `git_importer.rs`, `git_exporter.rs` |
| **Git Packfile V2** | `.pack` + `.idx` v2 files | Full (delta resolution) | Full (pack builder + fanout) | `pack.rs`, `pack_builder.rs`, `delta.rs` |
| **Git Fast Streams** | Fast-import stream (`.stream`, stdout) | Full streaming parser | Full streaming generator | `fast_stream.rs` |
| **Git LFS** | Git LFS pointers + media objects | Full pointer parsing & hydration | Full pointer & payload export | `lfs_interop.rs` |
| **GitHub Archive** | `.zip`, `.tar.gz` (repos + JSON metadata) | Full parsing & attribution | Target Spine format | `platform/models.rs`, `archive_security.rs` |
| **GitLab Archive** | `.tar.gz` (bundle + ndjson metadata) | Full parsing & attribution | Target Spine format | `platform/models.rs`, `archive_security.rs` |
| **Spine Native CAS** | `.spine/objects/`, `.spk` | Native storage | Native storage | `spine-object-store` |

---

## 2. PACKFILE V2 & DELTA SPECIFICATION DETAILS

### Packfile Header
```
[4 bytes: 'P', 'A', 'C', 'K']
[4 bytes: Version (0x00000002)]
[4 bytes: Object Count (u32 BE)]
```

### Pack Object Types
- `1`: `OBJ_COMMIT`
- `2`: `OBJ_TREE`
- `3`: `OBJ_BLOB`
- `4`: `OBJ_TAG`
- `6`: `OBJ_OFS_DELTA` (Offset delta: negative offset encoded as LEB128 relative to delta object position)
- `7`: `OBJ_REF_DELTA` (Reference delta: 20-byte base object SHA-1)

### Delta Instruction Opcodes
- **COPY:** MSB is set (`opcode & 0x80 != 0`). Next bits specify offset and size byte lengths. Copies a slice from the base object.
- **INSERT:** MSB is clear (`opcode & 0x80 == 0`). `opcode` specifies byte count of inline data to insert directly into target object.

---

## 3. INDEX FILE V2 (`.idx`) SPECIFICATION

1. **Header:** `\xFF\x74\x4F\x63` (`\377tOc`), followed by version `0x00000002`.
2. **Fanout Table:** 256 4-byte integers storing cumulative counts of objects whose SHA-1 starts with byte `0x00` through `0xFF`.
3. **Table of SHA-1s:** Sorted lexicographically, `20 * N` bytes.
4. **Table of CRC32s:** `4 * N` bytes of CRC32 checksums for each object in the pack.
5. **Table of Offsets:** `4 * N` bytes of 31-bit packfile offsets (or MSB-set pointers into 8-byte 64-bit offset table for large files).
6. **Trailers:**
   - 20-byte SHA-1 of the corresponding `.pack` file.
   - 20-byte SHA-1 of all preceding `.idx` bytes.

---

## 4. CANONICAL TREE SERIALIZATION FORMAT

A Git tree object contains sequential binary entries:
```
<mode_octal_ascii> <file_name>\0<20-byte-sha1-binary>
```
Example:
```
100644 README.md\0[20 bytes SHA-1]
040000 src\0[20 bytes SHA-1]
```
Entries **must** be sorted canonically as specified in Section 2.2 of `docs/GIT_OBJECT_MAPPING.md`.
