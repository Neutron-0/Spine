# Spine Forge: Branch Protection Rules and Governance Enforcement

## 1. Overview

Branch protection guards mission-critical branches (such as `main`, `master`, or release branches `release/*`) against accidental overwrites, unreviewed merges, force-pushes, and premature integrations.

---

## 2. Relational Schema

Defined in `migrations/20261103000002_forge_collaboration.sql`:

```sql
CREATE TABLE IF NOT EXISTS branch_protection_rules (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    branch_pattern VARCHAR(128) NOT NULL, -- e.g. 'main', 'release/*'
    require_pull_request BOOLEAN NOT NULL DEFAULT TRUE,
    required_approvals INT NOT NULL DEFAULT 1,
    prevent_self_approval BOOLEAN NOT NULL DEFAULT TRUE,
    require_changes_addressed BOOLEAN NOT NULL DEFAULT TRUE,
    enforce_for_admins BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, branch_pattern)
);
```

---

## 3. Pattern Matching Logic

Branch protection rules support both exact matches and wildcard prefix patterns:
- Exact: `main` matches branch `main`.
- Wildcard: `release/*` matches any branch starting with `release/`, such as `release/1.0`, `release/2026-v2`.

Implemented in `github-alternative/backend/apps/spine-api/src/permissions/service.rs`:

```rust
fn matches_pattern(pattern: &str, branch: &str) -> bool {
    if pattern == branch {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        if branch.starts_with(prefix) {
            return true;
        }
    }
    false
}
```

---

## 4. Merge Gatekeeper Verification

When a merge request is submitted to `POST /api/v1/repos/:owner/:repo/pulls/:number/merge`, the Gatekeeper verifies all active rules:

1. **Pull Request Required:** If direct push is attempted to a protected branch, it is forbidden.
2. **Approval Count:** The count of unique valid approvals from non-authors must be `>= rule.required_approvals`.
3. **No Active Changes Requested:** If any reviewer currently has a `CHANGES_REQUESTED` review that has not been superseded by an `APPROVED` review, merging is blocked with `400 Bad Request`.
4. **Self-Approval Check:** Approvals submitted by the PR author do not count towards the required threshold if `prevent_self_approval` is enabled.
5. **Addressed Feedback:** If `require_changes_addressed` is enabled and any inline review comments are unresolved (`resolved == false`), merging is rejected with a list of unaddressed comments.
6. **Admin Enforcement:** If `enforce_for_admins` is false, repository administrators can override branch protections. If true, even administrators must satisfy all review conditions.

---

## 5. API Endpoints

- `GET /api/v1/repos/:owner/:repo/protection`: List protection rules for the repository.
- `POST /api/v1/repos/:owner/:repo/protection`: Create or update a branch protection rule.
- `DELETE /api/v1/repos/:owner/:repo/protection/:rule_id`: Remove a branch protection rule.
