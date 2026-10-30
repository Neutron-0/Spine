# SPINE RESEARCH REFERENCE CORPUS

This directory houses technical references, source repositories, algorithmic analyses, and licensing inventories assembled for the architecture and implementation of **SPINE**.

## Directory Structure

```
references/
├── README.md                  # This document
├── REFERENCE_INVENTORY.md    # Catalog of cloned projects, commits, dates, scopes
├── LICENSE_INVENTORY.md      # Legal/intellectual boundary audit & license terms
├── RESEARCH_NOTES/           # In-depth architectural & algorithmic case studies
│   ├── 01_GIT_ANALYSIS.md
│   ├── 02_GITOXIDE_ANALYSIS.md
│   ├── 03_JUJUTSU_ANALYSIS.md
│   ├── 04_RADICLE_ANALYSIS.md
│   ├── 05_PIJUL_ANALYSIS.md
│   ├── 06_GIT_LFS_ANALYSIS.md
│   └── 07_GITEA_FORGEJO_ANALYSIS.md
└── sources/                  # Shallow-cloned upstream source trees (gitignored)
    ├── forgejo/              # Forgejo code hosting platform
    ├── git/                  # Official Git C implementation
    ├── git-lfs/              # Git Large File Storage Go client
    ├── gitea/                # Gitea code hosting platform
    ├── gitoxide/             # Pure Rust Git implementation
    ├── heartwood/            # Radicle Heartwood peer-to-peer code collaboration
    ├── jj/                   # Jujutsu VCS (Rust-native, operation log)
    └── pijul/                # Pijul patch-theory VCS (pijul-core v1.0.0-beta.26)
```

## Policy on Code Reuse & Licensing

1. **Isolation:** The `references/` directory is strictly isolated. It is never included in Cargo workspaces, crates, or frontend bundles.
2. **No Code Plagiarism or Copy-Pasting:** Spine is an original implementation. All source code in `git-alternative/` and `github-alternative/` is written from scratch specifically for Spine's architecture and data structures.
3. **Clean-Room Implementation:** Reference projects are analyzed for theoretical models, algorithmic tradeoffs, data format pitfalls, and protocol semantics.
