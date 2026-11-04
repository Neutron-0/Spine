# GIT LFS INTEROPERABILITY & LARGE OBJECT STORAGE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. GIT LFS SPECIFICATION SUPPORT

Git LFS (Large File Storage) replaces large binary files in a Git tree with small text pointer files of the form:
```text
version https://git-lfs.github.com/spec/v1
oid sha256:4b825dc642cb6eb9a060e54bf8d69288fbee4904ce2663040474e25ef6ef3940
size 12345678
```

Spine natively recognizes Git LFS pointers during import and export through `git-alternative/crates/spine-migration/src/lfs_interop.rs`.

---

## 2. INGESTION & HYDRATION ARCHITECTURE

When importing a Git repository containing LFS pointers:

1. **Pointer Detection:** The import scanner evaluates tree entries and file headers. If the file starts with `version https://git-lfs.github.com/spec/v1`, it is flagged as an LFS pointer.
2. **Payload Resolution:**  
   - If the Git repository includes the local LFS media store (`.git/lfs/objects/<prefix>/<sha256>`), Spine locates the actual binary payload.
   - The payload SHA-256 hash is computed and verified against the pointer's declared hash.
3. **Spine CAS Artifact Hydration:**  
   The binary payload is chunked and stored directly into Spine's distributed artifact store (`spine-artifact`) using content-defined chunking (CDC) and BLAKE3 / SHA-256 deduplication.
4. **Reference Preservation:**  
   In the native Spine repository, the file can be tracked as a native Spine LFS artifact or referenced directly by its chunk manifest.

---

## 3. EXPORTING TO GIT LFS

When exporting a Spine repository with large files or artifacts to Git:

1. Spine detects files exceeding the configured threshold (e.g., > 10 MB or matching `.gitattributes` large file rules).
2. Spine generates standard Git LFS pointer blobs in the Git tree.
3. Spine places the full binary payloads into the Git repository's `.git/lfs/objects/` store structured by SHA-256 prefix paths.
4. Git clients checking out the repository can run `git lfs pull` or `git checkout` seamlessly.

---

## 4. VERIFICATION EVIDENCE

Unit testing in `test_lfs_pointer_parsing_and_detection`:
- Correctly parses valid Git LFS v1 pointers with SHA-256 OIDs and byte lengths.
- Distinguishes between genuine LFS pointers and normal source code text files containing similar keywords.
- Successfully reconstructs standard LFS pointer format for export roundtrip.
