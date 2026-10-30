# SPINE

**Spine** is a high-performance, Rust-native distributed version-control ecosystem and modern developer collaboration platform.

Spine consists of two primary products:

1. **Spine VCS (`git-alternative/`):** An original, high-performance distributed version control system implemented in safe systems Rust, featuring:
   - Content-addressed Merkle DAG with domain-separated hashing (BLAKE3 default, SHA-256 compatibility).
   - High-throughput Zstandard compression with streaming chunk encoding.
   - Atomic lock-free and crash-resilient reference operations engineered for Windows, Linux, and macOS.
   - Native command-line interface (`spine`) with intuitive, predictable semantics (`init`, `config`, `add`, `status`, `commit`, `log`, `show`, `fsck`).
   - Clean library architecture separating core object models, storage engine, staging manifest, and history traversal.

2. **Spine Forge (`github-alternative/`):** A modern, resilient code-hosting and developer collaboration platform:
   - High-performance Rust backend built with Axum, Tokio, and PostgreSQL.
   - Provider-independent distributed storage architecture with decentralized storage nodes and cryptographic data integrity verification.
   - Clean, reactive TypeScript + React + Vite web interface.
   - Native Spine VCS protocol integration without relying on external Git subprocess wrappers.

---

## Workspace Structure

```
spine/
├── docs/                      # Architectural specifications, ADRs, benchmarks, format spec
├── git-alternative/           # Spine VCS engine, libraries, tests, and CLI binary
│   ├── apps/
│   │   └── spine-cli/         # `spine` CLI application
│   └── crates/
│       ├── spine-core/        # Object IDs, types, headers, traits
│       ├── spine-format/      # Serialization, binary encodings, domain separation
│       ├── spine-object-store/# Content-addressed storage, hashing, zstd compression
│       ├── spine-index/       # Staging manifest and index management
│       ├── spine-worktree/    # Filesystem scanning, ignore rules, status tracking
│       ├── spine-history/     # Commit graph traversal, topological log ordering
│       ├── spine-diff/        # Myers line diff and change calculation
│       ├── spine-config/      # User, repository, and global configuration
│       ├── spine-integrity/   # fsck, hash validation, dangling pointer checks
│       ├── spine-merge/       # 3-way tree & text merge engine, conflict markers
│       └── spine-journal/     # Append-only transaction log, undo/redo, crash recovery
├── github-alternative/        # Spine Forge collaboration platform
│   ├── backend/               # Rust Axum API services, distributed storage abstractions
│   └── frontend/              # React + TypeScript + Vite web client
└── references/                # Isolated research corpus and comparative evaluations
```

---

## Quick Start (Spine VCS CLI)

### Building
```powershell
cd git-alternative
cargo build --release
```

### Usage
```powershell
# Initialize a new Spine repository (.spine/)
spine init

# Set author configuration
spine config set user.name "Alice Engineer"
spine config set user.email "alice@example.com"

# Stage files and check status
spine add .
spine status

# Commit changes
spine commit -m "feat: initial commit"

# Inspect history and commits
spine log
spine show HEAD

# Branch management and switching
spine branch create feature-x
spine switch feature-x
spine branch

# Review diffs
spine diff
spine diff --staged --stat

# Three-way merge and conflict handling
spine switch main
spine merge feature-x
# If conflicts occur:
spine merge --abort      # or resolve files and:
spine merge --continue

# Transaction journal undo/redo
spine undo
spine redo
spine undo --list

# Verify repository integrity
spine fsck
```

---

## Engineering Principles

- **Zero Git Process Delegation:** Spine is not a wrapper around `git` or `libgit2`. It defines its own storage formats, object schemas, index formats, and engine primitives.
- **Safety and Reliability:** Built exclusively in safe Rust. Storage operations employ atomic tempfile swaps and transactional journal writes with explicit Windows file-lock handling.
- **Evidence-Based Performance:** Hashing (BLAKE3), compression (Zstandard), and traversal algorithms are benchmarked against standard datasets.
