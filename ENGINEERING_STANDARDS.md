# SPINE ENGINEERING STANDARDS

## 1. Rust Language Standards
- **Edition:** Rust 2021 edition (or modern stable equivalent).
- **Safety First:** Zero `unsafe` code in core business logic. If an algorithm requires `unsafe` for performance (e.g. SIMD vectorization or zero-copy memory mapping), it must be accompanied by an exhaustive `# Safety` contract and comparative benchmark justification.
- **Strict Linting:** All crates must pass `cargo clippy --all-targets -- -D warnings`.
- **Code Formatting:** All code formatted with standard `cargo fmt`.
- **Error Handling:** Standardized with `thiserror` for library error enumerations and `anyhow` for top-level CLI applications. Error messages must be actionable, conveying what happened, why, and how to resolve it.

## 2. Cross-Platform Guidelines
- **Windows as First-Class Citizen:**
  - Never assume POSIX path separators (`/`). Use `std::path::Path` and `PathBuf`.
  - In storage formats, normalize paths to UTF-8 forward slashes (`/`) for deterministic cross-platform object IDs.
  - Anticipate Windows file locking: Never attempt in-place file replacement of open handles. Use tempfiles and atomic renames.
  - Handle line endings gracefully (`\r\n` vs `\n`).
- **Linux & macOS:**
  - Preserve standard UNIX file modes (`0o644`, `0o755`).
  - Support symlinks safely: Never blindly dereference symlinks into parent or arbitrary system directories (path traversal protection).

## 3. Storage Integrity & Data Guarantees
- **Immutability:** Once written and verified, an object is never mutated.
- **Atomic Reference Updates:** References (`HEAD`, branch tips) must be updated atomically using temporary lockfiles or atomic renames.
- **Fail-Safe Crash Recovery:** If a process terminates unexpectedly during commit or staging, the repository must remain in a valid, uncorrupted state.
- **Strict Domain Separation:** Hashing inputs must always incorporate domain tags preventing cross-type collisions.

## 4. Performance Requirements
- **Content Hashing:** BLAKE3 by default for cryptographic-grade speed and tree hashing support.
- **Compression:** Zstandard for optimal decompression speed and customizable compression ratios.
- **Stat Cache Acceleration:** Staging checks must rely on cached stat comparisons (`mtime`, `size`, `inode`) to avoid re-reading disk contents on unmodified files.
