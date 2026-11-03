# Spine Forge: Notifications Engine and Activity Feed

## 1. Overview

Effective collaboration requires timely, contextual notification of relevant events (review requests, assignments, mentions, issue comments, and PR merges). Spine Forge provides a multi-channel notification subsystem paired with an immutable repository activity audit stream.

---

## 2. Relational Schema

Defined in `migrations/20261103000002_forge_collaboration.sql`:

```sql
CREATE TABLE IF NOT EXISTS user_notifications (
    id VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    title VARCHAR(256) NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    category VARCHAR(64) NOT NULL, -- 'REVIEW_REQUESTED', 'ASSIGNED', 'MENTIONED', 'MERGED', 'COMMENT'
    target_type VARCHAR(32) NOT NULL, -- 'ISSUE', 'PULL_REQUEST', 'COMMIT'
    target_id VARCHAR(64) NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS repository_activity_events (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    actor_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    event_type VARCHAR(64) NOT NULL, -- 'ISSUE_OPENED', 'PR_OPENED', 'PR_MERGED', 'REVIEW_SUBMITTED'
    payload JSONB NOT NULL,
    created_at_nanos BIGINT NOT NULL
);
```

---

## 3. Notification Dispatch Lifecycle

Events across the platform trigger notification generation asynchronously or inline:
- **Review Requests:** When a user is requested as a reviewer on a PR, a notification is dispatched with category `REVIEW_REQUESTED`.
- **Assignments:** When an issue or PR assignees list is updated, added assignees receive an `ASSIGNED` notification.
- **Mentions:** Parsing `@username` in markdown issue descriptions, PR bodies, or comments generates a `MENTIONED` notification to the referenced user.
- **PR Merges:** When a PR is merged, the PR author receives a `MERGED` notification, and authors of auto-closed issues receive completion alerts.

### Read State Management
Users can inspect unread notifications (`unread_only=true`), mark single notifications as read (`PATCH /api/v1/notifications/:id/read`), or clear all unread badges in bulk (`POST /api/v1/notifications/read-all`).

---

## 4. Repository Activity Stream

The `repository_activity_events` table forms an append-only timeline of collaboration events for each repository:
- `ISSUE_OPENED` / `ISSUE_CLOSED`
- `PR_OPENED` / `PR_MERGED` / `PR_CLOSED`
- `REVIEW_SUBMITTED`
- `COMMENT_ADDED`

The activity feed can be queried via `GET /api/v1/repos/:owner/:repo/activity`, providing a transparent dashboard for teams and audit compliance.
