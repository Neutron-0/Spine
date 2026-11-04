# Search and Discovery Architecture

Date: 2026-11-04  
System: SPINE Forge Platform  

---

## 1. Global & Code Search Architecture

### 1.1 Multi-Entity Search Model
Spine Forge implements an integrated search pipeline indexing five primary entity types:
1. **Repositories**: Name, normalized slug, description, and assigned topic tags.
2. **Code**: Tree blob content indexed with line numbers, file paths, and syntax recognition.
3. **Issues**: Titles and bodies.
4. **Pull Requests**: Titles, descriptions, and branch references.
5. **Discussions**: Thread titles and comment bodies.

### 1.2 Permission-Aware Filtering
Search queries execute under an authorization contract:
- Anonymous callers only observe hits from repositories with `is_private == false`.
- Authenticated callers observe public repositories plus private repositories where they hold active ownership, direct collaboration permissions, or team-level read access.
- Authorization gating occurs before returning results, preventing timing side-channel leaks or metadata leakage.

### 1.3 Code Snapshot Traversal
Code search directly inspects repository tree snapshots stored in native object stores:
- Avoids external dependencies or third-party wrappers.
- Filters out non-text files, hidden dot-directories (`.spine`, `.git`), build artifacts (`target/`), and dependency folders (`node_modules/`).
- Returns snippet context with matched line numbers and deep-link URLs (`/{owner}/{repo}/blob/main/{path}#L{line}`).

---

## 2. Topic Categorization & Language Analysis

### 2.1 Normalized Topics
- Topics are stored in `repository_topics` with a uniqueness constraint on `(repo_id, topic)`.
- Normalization strips whitespace, forces lowercase, and validates against the character set `[a-z0-9-]` (up to 35 characters).
- Popular topic aggregation powers community discovery feeds.

### 2.2 Polyglot Language Breakdown
- Analyzes repository directory structures on demand or upon push events.
- Maps file extensions to canonical language names (`.rs` -> Rust, `.ts` -> TypeScript, `.py` -> Python, `.go` -> Go, `.c`/`.h` -> C, `.cpp` -> C++, `.sql` -> SQL, etc.).
- Calculates total byte volume and fractional percentages per language.
- Persisted in `repository_languages` to provide instant rendering without on-the-fly filesystem scans.
