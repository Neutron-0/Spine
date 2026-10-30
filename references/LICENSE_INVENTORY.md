# LICENSE INVENTORY & INTELLECTUAL PROPERTY BOUNDARIES

This document catalogs the legal licenses governing all external reference materials inspected during Spine's research phase, documenting explicit boundaries to ensure clean-room engineering.

---

## 1. Reference Projects License Matrix

| Project | License Identifier | Copyleft / Permissive | Key Obligations | Spine Action & Architectural Separation |
|---|---|---|---|---|
| **Git** | GPL-2.0 | Strong Copyleft | Derivative works must be licensed under GPL-2.0 if distributed. | **No source reuse.** Inspected purely for understanding Git's packfile format and index format. Spine uses clean Rust code with distinct binary formats and domain-separated hashing. |
| **gitoxide** | Apache-2.0 / MIT | Permissive | Attribution notice required if code is copied. | **No source copied.** Inspected for Rust systems design patterns and performance considerations. |
| **Jujutsu (jj)** | Apache-2.0 | Permissive | Notice and license reproduction if code is reused. | **Clean-room adaptation.** Studied operation logging and working copy commit models. |
| **Radicle Heartwood**| Apache-2.0 / MIT | Permissive | Attribution notice required. | **Clean-room adaptation.** Studied P2P replication topology and decentralized identity schemas for Spine's distributed storage design. |
| **Pijul (pijul-core)**| GPL-2.0-or-later | Strong Copyleft | Derivative works must be GPL-2.0+. | **No source reuse.** Studied mathematical patch theory to compare with snapshot-based Merkle DAG. Spine adopts snapshot-oriented storage rather than Pijul's patch algebra. |
| **Git LFS** | MIT | Permissive | Attribution notice required. | **Specification analysis.** Studied pointer file format and batch transfer protocol for designing Spine's native large-file engine. |
| **Gitea** | MIT | Permissive | Attribution notice required. | **Architectural analysis.** Studied database schemas, permissions tables, and webhook dispatch queues for Spine Forge. |
| **Forgejo** | GPL-3.0-or-later | Strong Copyleft | Derivative works must be GPL-3.0+. | **No source reuse.** Studied runner protocol and storage drivers conceptually. |

---

## 2. Intellectual Property Assurance

1. **Independent Implementation:** All crates in `git-alternative/` and services in `github-alternative/` are authorial creations written specifically for Spine.
2. **Third-Party Dependencies:** Only standard, widely vetted open-source crates with permissive licenses (MIT, Apache-2.0, BSD-3-Clause, Zlib) are incorporated into Spine's Cargo workspaces:
   - `blake3` (Apache-2.0 / CC0)
   - `sha2` (Apache-2.0 / MIT)
   - `zstd` (MIT)
   - `serde` / `serde_json` (Apache-2.0 / MIT)
   - `tokio` (MIT)
   - `axum` (MIT)
   - `thiserror` / `anyhow` (Apache-2.0 / MIT)
   - `criterion` (Apache-2.0 / MIT)
3. **No GPL Contamination:** Neither Git, Pijul, nor Forgejo source code is linked, vendored, or compiled into any Spine binary or library.
