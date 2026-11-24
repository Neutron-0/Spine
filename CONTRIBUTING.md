# Contributing to Spine

Thank you for your interest in contributing to **Spine**! Spine is a high-performance distributed version control system and developer collaboration platform implemented in safe systems Rust.

Whether you are fixing a bug, improving documentation, optimizing storage performance, or proposing a new capability, we welcome your contributions.

---

## 1. Code of Conduct

All contributors and maintainers are expected to follow our [Code of Conduct](file:///d:/Vault/Spine/CODE_OF_CONDUCT.md). Please report any violations or unacceptable behavior through confidential GitHub reporting or security@spine-vcs.org.

---

## 2. Development Setup

### 2.1 Prerequisites
- **Rust Toolchain:** Rust stable (1.80+ or MSRV 1.97.1 per CI). Install via `rustup`.
  ```powershell
  rustup update stable
  rustup component add rustfmt clippy
  ```
- **PostgreSQL:** Version 16+ (or run via Docker Compose).
- **Docker & Docker Compose:** For running integration databases and full-stack local clusters.
- **Python:** Python 3.10+ (for CI verification scripts and security scanners).
- **Node.js:** Node.js 20+ and npm/yarn (for `github-alternative/frontend` development).

### 2.2 Workspace Architecture
Spine is organized as two independent Rust workspaces:
1. `git-alternative/`: Spine VCS engine, libraries, object store, and `spine` CLI binary.
2. `github-alternative/`: Spine Forge collaboration platform (Axum backend API, storage node RPC, React frontend).

---

## 3. Local Development Workflows

### 3.1 Building Spine VCS
```powershell
cd git-alternative
cargo build --all-targets
cargo test
```

### 3.2 Building Spine Forge Backend
```powershell
# Start local PostgreSQL and Redis dependencies
docker compose -f docker-compose.dev.yml up -d

# Run backend tests and services
cd github-alternative/backend
cargo build --all-targets
cargo test
```

### 3.3 Running Quality and Style Checks
Before submitting a pull request, ensure all linters, formatters, and static checks pass locally:

```powershell
# Format checks
cargo fmt --manifest-path git-alternative/Cargo.toml --all -- --check
cargo fmt --manifest-path github-alternative/backend/Cargo.toml --all -- --check

# Clippy lints (fail on warnings)
cargo clippy --manifest-path git-alternative/Cargo.toml --all-targets -- -D warnings
cargo clippy --manifest-path github-alternative/backend/Cargo.toml --all-targets -- -D warnings

# CI script and scanner verification
python -m pytest scripts/test_secret_scanner.py scripts/test_ci_change_scope.py scripts/test_ci_workflow_contract.py scripts/test_preflight_lfs_migration.py
```

---

## 4. Engineering Standards & Guidelines

### 4.1 Memory Safety and Zero-Copy I/O
- Spine core libraries strictly avoid `unsafe` code unless rigorously justified, audited, and isolated behind safe abstractions.
- All on-disk structures and protocol serializations must account for endianness and domain separation.

### 4.2 Security-First Coding
- **Path Sanitization:** File operations in working trees or repository storage must strictly sanitize paths against directory traversal (`..`, absolute paths, Windows alternate data streams `::$DATA`).
- **Secrets:** Never commit secrets, tokens, or live credentials. CI runs automated Gitleaks secret scanners that fail closed.
- **SQL Safety:** All database queries must use parameterized queries via SQLx; never construct dynamic raw SQL strings from untrusted input.

### 4.3 Git LFS and CAS Multi-Tenancy
- Storage blobs are content-addressed and deduplicated, but repository visibility is multi-tenant and strictly isolated.
- Any change touching LFS, CAS, or artifact endpoints must preserve repository-scoped authorization and mapping invariants (`lfs_objects` table).

---

## 5. Commit & Pull Request Guidelines

### 5.1 Commit Messages
We follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:
- `feat:` Introduces a new feature or capability.
- `fix:` Patches a bug or regression.
- `docs:` Documentation or diagram additions and updates.
- `refactor:` Code reorganization without behavioral change.
- `perf:` Performance optimizations and algorithmic improvements.
- `test:` Adding or refining unit, integration, or adversarial tests.
- `chore:` Dependency bumps, CI workflow tweaks, and toolchain updates.

### 5.2 Pull Request Checklist
When opening a pull request, please verify:
1. All unit and integration tests pass locally.
2. `cargo fmt` and `cargo clippy` report zero errors and zero warnings.
3. Relevant architectural changes or new APIs are documented in `ARCHITECTURE.md` or `docs/`.
4. Commit history is clean, atomic, and logically sequenced.
5. All CI checks on GitHub Actions succeed.

---

## 6. License

By contributing to Spine, you agree that your contributions will be licensed under its dual MIT and Apache-2.0 licenses.
