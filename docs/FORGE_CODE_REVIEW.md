# Spine Forge: Code Review Workflow and Inline Diff Comments

## 1. Overview

Code review is a cornerstone of collaborative software development in Spine Forge. Reviews allow peers to critique proposed changes, leave line-level feedback on unified diffs, request alterations, or formally approve integration.

---

## 2. Relational Schema

Defined in `migrations/20261103000002_forge_collaboration.sql`:

```sql
CREATE TABLE IF NOT EXISTS pull_request_reviews (
    id VARCHAR(64) PRIMARY KEY,
    pr_id VARCHAR(64) NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    author_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    state VARCHAR(32) NOT NULL, -- 'APPROVED', 'CHANGES_REQUESTED', 'COMMENTED'
    body TEXT NOT NULL DEFAULT '',
    commit_id VARCHAR(64) NOT NULL,
    submitted_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS pull_request_review_comments (
    id VARCHAR(64) PRIMARY KEY,
    review_id VARCHAR(64) NOT NULL REFERENCES pull_request_reviews(id) ON DELETE CASCADE,
    pr_id VARCHAR(64) NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    author_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    path VARCHAR(512) NOT NULL,
    line_number INT,
    diff_hunk TEXT NOT NULL DEFAULT '',
    body TEXT NOT NULL,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_by_user_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);
```

---

## 3. Review Verdicts and Resolution Invariants

A formal review submission includes one of three states:
1. **`APPROVED`:** Reviewer certifies that code is safe and ready to merge.
2. **`CHANGES_REQUESTED`:** Reviewer blocks merging until specified issues or requested improvements are resolved.
3. **`COMMENTED`:** General feedback or questions without blocking or approving.

### Self-Approval Prevention
By default, and strictly enforced under branch protection rules, a pull request author cannot submit an `APPROVED` review on their own pull request. Submissions attempting this are rejected with `400 Bad Request` or ignored during merge eligibility checks.

### Inline Comments & Resolution Tracking
Comments anchored to a specific file and line number (`path` and `line_number`) can be resolved individually (`resolved: true`). When branch protection has `require_changes_addressed: true`, un-resolved inline comments block merge execution until marked resolved by the author or reviewer.

---

## 4. API Endpoints

- `GET /api/v1/repos/:owner/:repo/pulls/:number/reviews`: List reviews for a PR.
- `POST /api/v1/repos/:owner/:repo/pulls/:number/reviews`: Submit a review (`APPROVED`, `CHANGES_REQUESTED`, `COMMENTED`).
- `GET /api/v1/repos/:owner/:repo/pulls/:number/comments`: List inline review comments.
- `POST /api/v1/repos/:owner/:repo/pulls/:number/comments`: Post an inline review comment.
- `PATCH /api/v1/repos/:owner/:repo/pulls/:number/comments/:comment_id/resolve`: Toggle resolved status of an inline comment.
