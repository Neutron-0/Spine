# Spine Forge: Issues, Labels, Milestones, and Discussion Threads

## 1. Overview

Spine Forge provides a native work-tracking subsystem integrated directly with the repository history. Issues represent bug reports, tasks, and feature proposals. They support hierarchical metadata including assignees, scoped color-coded labels, milestone progress tracking, and chronological discussion comments.

---

## 2. Relational Schema

Defined in `migrations/20261103000002_forge_collaboration.sql`:

```sql
CREATE TABLE IF NOT EXISTS milestones (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    title VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    state VARCHAR(32) NOT NULL DEFAULT 'OPEN', -- 'OPEN', 'CLOSED'
    due_date_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, title)
);

CREATE TABLE IF NOT EXISTS repository_labels (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    name VARCHAR(64) NOT NULL,
    color VARCHAR(16) NOT NULL, -- e.g. '#d73a4a'
    description VARCHAR(256) NOT NULL DEFAULT '',
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, name)
);

CREATE TABLE IF NOT EXISTS issues (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    issue_number INT NOT NULL,
    author_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    title VARCHAR(256) NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    status VARCHAR(32) NOT NULL DEFAULT 'OPEN', -- 'OPEN', 'CLOSED'
    milestone_id VARCHAR(64) REFERENCES milestones(id) ON DELETE SET NULL,
    closed_at_nanos BIGINT,
    closed_by_user_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, issue_number)
);

CREATE TABLE IF NOT EXISTS issue_assignees (
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    PRIMARY KEY (issue_id, user_id)
);

CREATE TABLE IF NOT EXISTS issue_labels (
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    label_id VARCHAR(64) NOT NULL REFERENCES repository_labels(id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    PRIMARY KEY (issue_id, label_id)
);

CREATE TABLE IF NOT EXISTS issue_comments (
    id VARCHAR(64) PRIMARY KEY,
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);
```

---

## 3. Auto-Closing Semantics via Pull Requests

When pull requests are merged, commit messages and PR bodies are parsed for closing keywords matching standard issue references:
- `Fixes #<number>`
- `Fix #<number>`
- `Closes #<number>`
- `Close #<number>`
- `Resolves #<number>`
- `Resolve #<number>`

Upon an atomic merge execution in `storage/vcs.rs` / `routes_pulls.rs`:
1. The regex pattern `(?i)(?:fixes|fixed|fix|closes|closed|close|resolves|resolved|resolve)\s+#(\d+)` extracts all referenced issue numbers.
2. For each referenced issue in the repository, the issue status is transitioned to `CLOSED`.
3. `closed_by_user_id` is set to the merging user, and `closed_at_nanos` records the exact merge timestamp.
4. A notification is dispatched to the issue author, and a `RepositoryActivityEvent` is appended to the audit log.

---

## 4. API Endpoints

- `GET /api/v1/repos/:owner/:repo/issues`: List issues (filtered by `status=OPEN|CLOSED`, `milestone`, `labels`).
- `POST /api/v1/repos/:owner/:repo/issues`: Create an issue.
- `GET /api/v1/repos/:owner/:repo/issues/:number`: Fetch issue details, labels, assignees, and milestone.
- `PATCH /api/v1/repos/:owner/:repo/issues/:number`: Update issue title, body, status, assignees, or milestone.
- `GET /api/v1/repos/:owner/:repo/issues/:number/comments`: List comments for an issue.
- `POST /api/v1/repos/:owner/:repo/issues/:number/comments`: Post a comment to an issue.
- `GET /api/v1/repos/:owner/:repo/labels`: List repository labels.
- `POST /api/v1/repos/:owner/:repo/labels`: Create a label.
- `GET /api/v1/repos/:owner/:repo/milestones`: List repository milestones.
- `POST /api/v1/repos/:owner/:repo/milestones`: Create a milestone.
