# SEARCH & INDEX CONSISTENCY SPECIFICATION

**Date:** 2026-11-04  
**Batch Milestone:** Batch 14 — Full-System Integration, Security Hardening, Reliability, Recovery, and Performance  
**Auditor / Engineering Lead:** DeepMind Advanced Agentic Coding Pair Program  
**Workspace:** `d:\Vault\Spine`

---

## 1. PERMISSION-AWARE QUERY FILTERING

Search must never become an authorization bypass. Query-time authorization filtering is enforced directly inside `search/service.rs`:

1. **Anonymous Queries:** Only public repositories (`repo.is_private == false`) and public topics are queried. Private repositories, code snippets, discussions, and commit messages are excluded before scoring.
2. **Authenticated Queries:**
   - Evaluated via `PermissionService::can_read_repo(actor_id, repo_id)`.
   - Only repositories where the actor has `Owner`, `Admin`, `Maintainer`, `Write`, or `Read` access are permitted in search results.
3. **Visibility Transitions:** If a repository transitions from public to private, subsequent anonymous queries immediately cease returning hits without requiring an index rebuild because permission checks evaluate current database access state at query time.
4. **Deleted Repositories:** Handled immediately; deleted repositories are omitted from `list_public_repositories` and return `NotFound` or `Unauthorized` on access.

---

## 2. RESOURCE SAFETY & BOUNDED BUFFERS

1. **Query Length Bounds:** Search queries are trimmed and bounded to 256 characters.
2. **Snippet Size Limits:** Code search snippets extract at most 1 context line before and after matching lines, with maximum line lengths bounded to 200 characters to prevent memory amplification.
3. **File Exclusion:** Binary files, files larger than 1 MB, `target/`, `.spine/`, and `node_modules/` are automatically skipped during code search indexing.
4. **Pagination:** Results are capped at 50 hits per query page.
