# INTEROPERABILITY FIDELITY MATRIX

**Project:** Spine  
**Batch Milestone:** Batch 15 — Native Git Interoperability, Repository Import/Export, Platform Migration, and Ecosystem Compatibility  
**Author:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. PURPOSE & FIDELITY DEFINITIONS

This matrix establishes the fidelity tier of all version-control concepts when translating between Git, external developer platforms (GitHub, GitLab), and native Spine repositories.

### Fidelity Tiers

- **T1: Bit-for-Bit Identical:** Content bytes, names, permissions, and directory trees are preserved without modification.
- **T2: Semantically Equivalent:** Concepts exist in both systems and are fully translated without information loss, though internal cryptographic identifiers or serialization framing differ (e.g. Git SHA-1 vs. Spine BLAKE3).
- **T3: Attributed Adaptation:** Concept is adapted to Spine's native data model with security sanitization or administrative demotion (e.g. external users mapped to ghost tombstones).
- **T4: Safe Demotion / Fail-Closed:** Active automated features (e.g. CI/CD actions) are imported in a deactivated state (`is_active: false`) to safeguard against remote code execution.

---

## 2. REPOSITORY & VCS OBJECT MATRIX

| Git Concept | Spine Equivalent | Fidelity Tier | Notes |
|---|---|---|---|
| **Blob (File Content)** | `LooseObject::Blob` | **T1** | Byte-for-byte identical file payloads. |
| **Executable Bit (`100755`)** | `FileMode::Executable` | **T1** | POSIX executable permission bit preserved. |
| **Symlink (`120000`)** | `FileMode::Symlink` | **T1** | Symlink target path string preserved. |
| **Tree (Directory Structure)** | `LooseObject::Tree` | **T2** | Hierarchical directory structure preserved; canonical sorting enforced. |
| **Submodule Pointer (`160000`)**| `FileMode::Submodule` | **T2** | Git commit OID recorded in submodule entry. |
| **Commit Message** | `Commit::message` | **T1** | UTF-8 commit message preserved verbatim. |
| **Author Name & Email** | `Commit::author` | **T1** | Preserved verbatim. |
| **Author Timestamp & Offset** | `Commit::timestamp` | **T1** | Unix epoch seconds and timezone offset preserved. |
| **Committer Identity & Time** | `Commit::committer` | **T1** | Preserved verbatim. |
| **Commit Ancestry (Parents)** | `Commit::parents` | **T2** | Parent relationships topologically translated. |
| **Lightweight Tag** | `Ref (refs/tags/*)` | **T2** | Direct reference pointing to target commit. |
| **Annotated Tag** | `AnnotatedTag` | **T2** | Tagger identity, timestamp, and message preserved. |
| **GPG/PGP Signature** | `Commit::signature` | **T2** | Armored ASCII signature payload preserved as metadata. |
| **Packfile v2 (`.pack`)** | `.spk` Packfile | **T2** | Deflated pack translated to Spine zstd packfile or loose objects. |
| **Pack Index (`.idx`)** | Multi-pack Index | **T2** | Fanout and offset table translated into Spine index. |
| **Git LFS Pointers** | Spine CAS Artifact Store | **T2** | Pointers detected, verified, and hydrated to Spine CAS chunks. |

---

## 3. PLATFORM COLLABORATION ARTIFACTS MATRIX

| External Platform Concept | Spine Forge Equivalent | Fidelity Tier | Notes |
|---|---|---|---|
| **Issues** | Forge Issues | **T2** | Title, body, open/closed status, timestamps preserved. |
| **Pull Requests / Merge Requests** | Forge Pull Requests | **T2** | Source/target branch, title, description, state preserved. |
| **Issue / PR Comments** | Forge Comments | **T2** | Chronological comment threads preserved. |
| **Labels & Colors** | Forge Labels | **T1** | Name, 6-hex RGB color, and descriptions preserved. |
| **Milestones** | Forge Milestones | **T2** | Due date, title, state, and description preserved. |
| **Releases & Binary Assets** | Forge Releases | **T2** | Tag name, release notes, and artifact attachments preserved. |
| **User Accounts** | Ghost Users (`ghost-<name>`) | **T3** | Safe tombstones created; zero repo admin privileges granted. |
| **CI/CD Workflows (`.github/workflows`)**| Forge Workflows (`.spine/workflows`) | **T4** | Syntax parsed; migrated with `is_active: false` (fail-closed). |
| **Protected Branches** | Branch Protections | **T2** | Force-push prevention and required reviews mapped to Forge rules. |

---

## 4. ARCHIVE FORMAT SUPPORT MATRIX

| Format | Extension | Defenses Applied | Max Archive Size | Max File Count | Max Ratio |
|---|---|---|---|---|---|
| **Standard Zip** | `.zip` | Zip Slip rejection, Windows device check | 10 GB uncompressed | 1,000,000 files | 100:1 |
| **Tarball** | `.tar` | Path traversal rejection, device name check | 10 GB uncompressed | 1,000,000 files | N/A |
| **Gzipped Tarball** | `.tar.gz`, `.tgz` | Decompression limit, Zip Slip rejection | 10 GB uncompressed | 1,000,000 files | 100:1 |
| **Bare Git Archive** | `.git/` folder | Direct filesystem traversal and validation | Bound by filesystem | Unlimited | N/A |
