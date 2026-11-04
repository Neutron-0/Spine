# NATIVE GIT IMPORT GUIDE

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. OVERVIEW

Spine provides direct, native import of Git repositories without requiring Git to be installed on your machine. The import engine reads bare Git repositories, standard Git working directories, and fast-import streams directly from disk into Spine's content-addressed `.spine/` format.

---

## 2. USING THE SPINE CLI

### 2.1 Importing a Git Repository Directly

To import a Git repository into a native Spine repository:

```bash
# General syntax:
spine import git <path-to-git-repo> --target <path-to-target-spine-repo>

# Example importing a local working directory repository:
spine import git /d/projects/my-git-app --target /d/projects/my-spine-app

# Example importing a bare Git repository:
spine import git /d/repos/repo.git --target /d/projects/my-spine-app
```

### 2.2 What Happens During Import

1. **Discovery:** The engine checks for either `.git/` or a bare Git repository structure containing `objects/`, `refs/`, and `HEAD`.
2. **Object Ingestion:** Reads all loose objects in `objects/??/` and all packfiles in `objects/pack/`. Resolves delta chains (`OFS_DELTA` and `REF_DELTA`).
3. **Topology Conversion:**
   - Git Blobs are written as Spine CAS Blobs.
   - Git Trees are translated to Spine Trees, maintaining folder hierarchies.
   - Git Commits are converted topologically, preserving author, committer, timestamps, and commit messages.
   - Annotated Tags and lightweight references are created under `.spine/refs/`.
4. **Working Copy Checkout:** If importing into a non-bare Spine repository, the `HEAD` commit is checked out into the working tree.

---

## 3. IMPORTING VIA SPINE FORGE WEB UI

1. Open the **Spine Forge** interface in your browser.
2. Navigate to the **Migration** tab in the top navigation bar.
3. Select **Git Repository** as the migration source kind.
4. Enter the source directory path and target repository name.
5. Click **Inspect Source**: Spine inspects the repository, reporting total objects, commits, branches, and tags.
6. Click **Generate Plan**: Spine validates storage requirements, branch mappings, and checks for potential naming conflicts.
7. Click **Run Migration**: The backend executes the migration and reports real-time progress and final statistics.

---

## 4. ADVANCED: FAST-IMPORT STREAMS

For specialized migrations (e.g. from Mercurial, SVN, or custom VCS), Spine supports streaming import via Git fast-import syntax:

```bash
spine import stream <export.stream> --target <path-to-spine-repo>
```

Supported fast-import commands:
- `blob`: Inlines or consumes raw file contents.
- `commit`: Creates commits with author, committer, and file modifications (`M`, `D`).
- `tag`: Creates annotated tags with message payloads.
- `reset`: Sets branch references.
- `checkpoint`: Flushes intermediate transaction states.
