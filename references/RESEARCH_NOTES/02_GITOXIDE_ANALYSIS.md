# RESEARCH NOTE 02: GITOXIDE (GIX) ARCHITECTURE ANALYSIS

## 1. Architectural Philosophy
`gitoxide` is an idiomatic Rust re-implementation of Git focused on speed, safety, and modularity.
- Deconstructs Git into granular crates: `gix-hash`, `gix-object`, `gix-odb`, `gix-pack`, `gix-index`, `gix-worktree`, `gix-ref`.
- Eliminates allocations wherever possible using zero-copy byte slices (`&[u8]`) for object headers and parsing.
- Employs parallel Rayon iterators for directory scanning and packfile decoding.

## 2. Key Subsystems Inspected
- `gix-odb`: Object Database abstraction. Separates loose storage, packfiles, and alternates into composable layers.
- `gix-index`: Memory-mapped parsing of the Git index file. Parses 100k entries in ~15ms on modern hardware.
- `gix-worktree`: Filesystem status comparison using stat cache, parallel file attribute resolution, and path normalization.

## 3. What Spine Adopts
1. **Granular Crate Architecture:** Decoupling core primitives (`spine-core`, `spine-format`) from storage (`spine-object-store`) and CLI porcelain (`spine-cli`).
2. **Buffer Reuse & Minimal Allocations:** Avoiding heap churn during tree parsing and status checks.
3. **Explicit Cross-Platform Path Primitives:** Using normalized relative paths with forward slashes internally.

## 4. What Spine Avoids
1. **Emulation of Git Quirks:** `gitoxide` must faithfully emulate decades of Git edge cases and bugs (e.g., path casing issues, index v2/v3/v4 quirks). Spine defines its own clean, unambiguous format specifications.
