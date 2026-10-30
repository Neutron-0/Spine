# RESEARCH NOTE 01: GIT ARCHITECTURE & IMPLEMENTATION ANALYSIS

## 1. Storage & Object Model
Git models repository history as a directed acyclic graph (DAG) of content-addressed immutable objects:
- **Loose Objects:** Stored in `.git/objects/xx/yyy...` where `xx` is the first 2 hex digits of the 160-bit SHA-1 (or 256-bit SHA-256) hash, and `yyy...` is the remaining hash. Content is formatted as `<type> <size>\0<payload>` and compressed with zlib.
- **Tree Objects:** Hierarchical lists of entries (`<mode> <path>\0<hash_bytes>`). A directory tree is fully serialized at each level.
- **Packfiles & Delta Compression:** To prevent storage explosion from millions of loose objects, Git packs objects into `.pack` files with `.idx` lookup tables. Objects are delta-compressed against sliding windows of similar objects.
- **Commit Graph (`commit-graph`):** Auxiliary cache storing generation numbers and topology for $O(1)$ graph traversal.

## 2. Key Subsystems Inspected
- `read-cache.c`: The binary index (`.git/index`). Contains stat cache entries (`ctime`, `mtime`, `dev`, `ino`, `mode`, `uid`, `gid`, `file_size`, `sha1`, flags, path). Enables instantaneous change detection without reading unchanged file payloads.
- `cache-tree.c`: Accelerates commit generation by caching pre-computed tree object IDs for unchanged directories in the index.
- `sha1-file.c` / `object-file.c`: Loose object write path. Employs temporary files in `.git/objects/` with atomic rename.

## 3. What Spine Adopts
1. **Content-addressed Merkle tree snapshots:** Incomparably clean, robust deduplication, and verifiable history.
2. **Stat-cache binary index:** Critical for achieving sub-10ms `spine status` on repositories with 50,000+ files.
3. **Atomic write-and-rename:** Guarantees crash-safety and immunity to half-written objects.

## 4. What Spine Avoids
1. **Undifferentiated object headers:** Git lacks explicit domain separation in its core hash calculations, relying on `<type> <size>\0`, which makes formal verification and multi-hash transitions harder.
2. **Legacy zlib compression:** Zstandard achieves 3-5x faster decompression and superior compression ratios.
3. **Complex C pointer semantics & global state:** Git's C codebase relies heavily on global state (e.g. `the_index`), complicating multi-threaded operations. Spine uses pure safe Rust with explicit thread-safe structures.
