-- 0004_issues_and_pull_requests.sql
-- Labels, milestones, issues, issue assignees/labels/comments, pull requests, reviewers, reviews, review comments.

CREATE TABLE IF NOT EXISTS repository_labels (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#0969da',
    description TEXT,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_label UNIQUE (repo_id, name)
);

CREATE INDEX IF NOT EXISTS idx_labels_repo ON repository_labels (repo_id);

CREATE TABLE IF NOT EXISTS milestones (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    description TEXT,
    due_date_nanos BIGINT,
    state TEXT NOT NULL DEFAULT 'OPEN',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_milestones_repo ON milestones (repo_id);

CREATE TABLE IF NOT EXISTS issues (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    issue_number BIGINT NOT NULL,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'OPEN',
    milestone_id TEXT REFERENCES milestones (id) ON DELETE SET NULL,
    closed_by_user_id TEXT REFERENCES users (id) ON DELETE SET NULL,
    closed_at_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_issue_num UNIQUE (repo_id, issue_number)
);

CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues (repo_id);
CREATE INDEX IF NOT EXISTS idx_issues_author ON issues (author_id);
CREATE INDEX IF NOT EXISTS idx_issues_status ON issues (repo_id, status);

CREATE TABLE IF NOT EXISTS issue_assignees (
    issue_id TEXT NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, user_id)
);

CREATE TABLE IF NOT EXISTS issue_labels (
    issue_id TEXT NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES repository_labels (id) ON DELETE CASCADE,
    PRIMARY KEY (issue_id, label_id)
);

CREATE TABLE IF NOT EXISTS issue_comments (
    id TEXT PRIMARY KEY,
    issue_id TEXT NOT NULL REFERENCES issues (id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    body TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_issue_comments_issue ON issue_comments (issue_id);

CREATE TABLE IF NOT EXISTS pull_requests (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    pr_number BIGINT NOT NULL,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    base_branch TEXT NOT NULL,
    head_branch TEXT NOT NULL,
    base_commit_id TEXT NOT NULL,
    head_commit_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'OPEN',
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    merged_by_user_id TEXT REFERENCES users (id) ON DELETE SET NULL,
    merge_commit_id TEXT,
    merge_strategy TEXT,
    closed_at_nanos BIGINT,
    merged_at_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_pr_num UNIQUE (repo_id, pr_number)
);

CREATE INDEX IF NOT EXISTS idx_prs_repo ON pull_requests (repo_id);
CREATE INDEX IF NOT EXISTS idx_prs_author ON pull_requests (author_id);
CREATE INDEX IF NOT EXISTS idx_prs_state ON pull_requests (repo_id, state);

CREATE TABLE IF NOT EXISTS pull_request_reviewers (
    pr_id TEXT NOT NULL REFERENCES pull_requests (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    PRIMARY KEY (pr_id, user_id)
);

CREATE TABLE IF NOT EXISTS pull_request_reviews (
    id TEXT PRIMARY KEY,
    pr_id TEXT NOT NULL REFERENCES pull_requests (id) ON DELETE CASCADE,
    reviewer_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    commit_id TEXT NOT NULL,
    state TEXT NOT NULL,
    body TEXT,
    submitted_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_reviews_pr ON pull_request_reviews (pr_id);

CREATE TABLE IF NOT EXISTS pull_request_review_comments (
    id TEXT PRIMARY KEY,
    review_id TEXT NOT NULL REFERENCES pull_request_reviews (id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    path TEXT NOT NULL,
    line_number INT,
    diff_hunk TEXT,
    body TEXT NOT NULL,
    in_reply_to_comment_id TEXT REFERENCES pull_request_review_comments (id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_review_comments_review ON pull_request_review_comments (review_id);
