# NATIVE GIT EXPORT GUIDE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. ZERO VENDOR LOCK-IN PHILOSOPHY

Spine is committed to absolute developer freedom and data sovereignty. Any repository hosted or tracked in Spine can be exported at any time into a standard Git repository that is 100% compliant with the official Git specification and verifiable by `git fsck --full`.

Spine does not call external Git to perform this export—the export engine compiles pure Git loose objects, Packfiles v2, and `.idx` fanout tables in Rust.

---

## 2. USING THE SPINE CLI

### 2.1 Exporting to a Git Working Copy or Bare Repo

```bash
# General syntax:
spine export git <path-to-spine-repo> --target <path-to-output-git-repo>

# Example:
spine export git /d/projects/my-spine-app --target /d/exports/my-git-app
```

### 2.2 Output Structure

Spine creates a standard Git repository format:
```
my-git-app/
├── .git/
│   ├── HEAD                     # ref: refs/heads/main
│   ├── config                   # Standard git config
│   ├── objects/
│   │   ├── pack/                # Optional packfile and .idx v2
│   │   │   ├── pack-<hash>.pack
│   │   │   └── pack-<hash>.idx
│   │   └── ??/                  # Loose objects
│   └── refs/
│       ├── heads/
│       │   └── main
│       └── tags/
└── <checked-out files>
```

---

## 3. EXPORTING VIA SPINE FORGE WEB UI & API

### 3.1 Via Web UI
1. Navigate to the **Migration** tab in Spine Forge.
2. Select the **Export to Git** tab.
3. Choose the Spine repository you wish to export and specify the destination path.
4. Click **Run Export**.
5. Once completed, a summary displays the exported commit count, object count, branches, and tags.

### 3.2 Via REST API
```bash
POST /api/v1/migration/export
Content-Type: application/json
Authorization: Bearer <token>

{
  "source_repo": "alice/my-spine-project",
  "target_path": "/d/exports/my-git-app",
  "format": "git"
}
```

---

## 4. VERIFYING EXPORTED REPOSITORIES

You can verify the exported repository with standard Git tooling:

```bash
cd /d/exports/my-git-app

# 1. Full database integrity check
git fsck --full --strict

# 2. View commit history
git log --graph --oneline --decorate --all

# 3. Verify file contents at HEAD
git status
```
All commands will execute with 0 errors or warnings.
