-- ==============================================================================
-- Spine Forge: Collaboration, Organizations, Teams, Issues, Pull Requests, Reviews,
-- Branch Protection, and Notifications
-- Migration: 20261103000002_forge_collaboration.sql
-- ==============================================================================

-- 1. Organizations
CREATE TABLE IF NOT EXISTS organizations (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(100) NOT NULL,
    normalized_name VARCHAR(100) NOT NULL UNIQUE,
    display_name VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    avatar_url TEXT,
    owner_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_org_owner ON organizations(owner_user_id);
CREATE INDEX IF NOT EXISTS idx_org_normalized_name ON organizations(normalized_name);

-- 2. Organization Memberships
CREATE TABLE IF NOT EXISTS organization_memberships (
    id VARCHAR(64) PRIMARY KEY,
    org_id VARCHAR(64) NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role VARCHAR(32) NOT NULL DEFAULT 'MEMBER',
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (org_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_org_members_org ON organization_memberships(org_id);
CREATE INDEX IF NOT EXISTS idx_org_members_user ON organization_memberships(user_id);

-- 3. Organization Invitations
CREATE TABLE IF NOT EXISTS organization_invitations (
    id VARCHAR(64) PRIMARY KEY,
    org_id VARCHAR(64) NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    inviter_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    invitee_email VARCHAR(255) NOT NULL,
    role VARCHAR(32) NOT NULL DEFAULT 'MEMBER',
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    responded_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_org_invitations_org ON organization_invitations(org_id);
CREATE INDEX IF NOT EXISTS idx_org_invitations_token ON organization_invitations(token_hash);
CREATE INDEX IF NOT EXISTS idx_org_invitations_email ON organization_invitations(invitee_email);

-- 4. Teams
CREATE TABLE IF NOT EXISTS teams (
    id VARCHAR(64) PRIMARY KEY,
    org_id VARCHAR(64) NOT NULL REFERENCES organizations(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    normalized_slug VARCHAR(100) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (org_id, normalized_slug)
);

CREATE INDEX IF NOT EXISTS idx_teams_org ON teams(org_id);

-- 5. Team Memberships
CREATE TABLE IF NOT EXISTS team_memberships (
    id VARCHAR(64) PRIMARY KEY,
    team_id VARCHAR(64) NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    is_maintainer BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (team_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_team_members_team ON team_memberships(team_id);
CREATE INDEX IF NOT EXISTS idx_team_members_user ON team_memberships(user_id);

-- 6. Team Repository Permissions
CREATE TABLE IF NOT EXISTS team_repository_permissions (
    id VARCHAR(64) PRIMARY KEY,
    team_id VARCHAR(64) NOT NULL REFERENCES teams(id) ON DELETE CASCADE,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    permission_level VARCHAR(32) NOT NULL DEFAULT 'READ',
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (team_id, repo_id)
);

CREATE INDEX IF NOT EXISTS idx_team_repo_perm_team ON team_repository_permissions(team_id);
CREATE INDEX IF NOT EXISTS idx_team_repo_perm_repo ON team_repository_permissions(repo_id);

-- 7. Add owner_org_id to forge_repositories
ALTER TABLE forge_repositories ADD COLUMN IF NOT EXISTS owner_org_id VARCHAR(64) REFERENCES organizations(id) ON DELETE RESTRICT;
CREATE INDEX IF NOT EXISTS idx_forge_repos_org ON forge_repositories(owner_org_id);

-- 8. Milestones
CREATE TABLE IF NOT EXISTS milestones (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    title VARCHAR(255) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    state VARCHAR(32) NOT NULL DEFAULT 'OPEN',
    due_date_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    closed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_milestones_repo ON milestones(repo_id);

-- 9. Repository Labels
CREATE TABLE IF NOT EXISTS repository_labels (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    name VARCHAR(100) NOT NULL,
    color VARCHAR(16) NOT NULL DEFAULT '#58a6ff',
    description TEXT NOT NULL DEFAULT '',
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, name)
);

CREATE INDEX IF NOT EXISTS idx_repo_labels_repo ON repository_labels(repo_id);

-- 10. Issues
CREATE TABLE IF NOT EXISTS issues (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    issue_number BIGINT NOT NULL,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    title VARCHAR(255) NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    status VARCHAR(32) NOT NULL DEFAULT 'OPEN',
    milestone_id VARCHAR(64) REFERENCES milestones(id) ON DELETE SET NULL,
    closed_by_user_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    closed_at_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, issue_number)
);

CREATE INDEX IF NOT EXISTS idx_issues_repo ON issues(repo_id);
CREATE INDEX IF NOT EXISTS idx_issues_author ON issues(author_id);
CREATE INDEX IF NOT EXISTS idx_issues_status ON issues(repo_id, status);

-- 11. Issue Assignees
CREATE TABLE IF NOT EXISTS issue_assignees (
    id VARCHAR(64) PRIMARY KEY,
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (issue_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_issue_assignees_issue ON issue_assignees(issue_id);
CREATE INDEX IF NOT EXISTS idx_issue_assignees_user ON issue_assignees(user_id);

-- 12. Issue Labels
CREATE TABLE IF NOT EXISTS issue_labels (
    id VARCHAR(64) PRIMARY KEY,
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    label_id VARCHAR(64) NOT NULL REFERENCES repository_labels(id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (issue_id, label_id)
);

CREATE INDEX IF NOT EXISTS idx_issue_labels_issue ON issue_labels(issue_id);
CREATE INDEX IF NOT EXISTS idx_issue_labels_label ON issue_labels(label_id);

-- 13. Issue Comments
CREATE TABLE IF NOT EXISTS issue_comments (
    id VARCHAR(64) PRIMARY KEY,
    issue_id VARCHAR(64) NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    body TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_issue_comments_issue ON issue_comments(issue_id);
CREATE INDEX IF NOT EXISTS idx_issue_comments_author ON issue_comments(author_id);

-- 14. Pull Requests
CREATE TABLE IF NOT EXISTS pull_requests (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    pr_number BIGINT NOT NULL,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    title VARCHAR(255) NOT NULL,
    body TEXT NOT NULL DEFAULT '',
    base_branch VARCHAR(255) NOT NULL,
    head_branch VARCHAR(255) NOT NULL,
    base_commit_id VARCHAR(64) NOT NULL,
    head_commit_id VARCHAR(64) NOT NULL,
    state VARCHAR(32) NOT NULL DEFAULT 'OPEN',
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    merged_by_user_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    merge_commit_id VARCHAR(64),
    merge_strategy VARCHAR(32) DEFAULT 'MERGE_COMMIT',
    closed_at_nanos BIGINT,
    merged_at_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, pr_number)
);

CREATE INDEX IF NOT EXISTS idx_prs_repo ON pull_requests(repo_id);
CREATE INDEX IF NOT EXISTS idx_prs_author ON pull_requests(author_id);
CREATE INDEX IF NOT EXISTS idx_prs_state ON pull_requests(repo_id, state);

-- 15. Pull Request Reviewers
CREATE TABLE IF NOT EXISTS pull_request_reviewers (
    id VARCHAR(64) PRIMARY KEY,
    pr_id VARCHAR(64) NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    reviewer_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    requested_by_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE (pr_id, reviewer_id)
);

CREATE INDEX IF NOT EXISTS idx_pr_reviewers_pr ON pull_request_reviewers(pr_id);
CREATE INDEX IF NOT EXISTS idx_pr_reviewers_user ON pull_request_reviewers(reviewer_id);

-- 16. Pull Request Reviews
CREATE TABLE IF NOT EXISTS pull_request_reviews (
    id VARCHAR(64) PRIMARY KEY,
    pr_id VARCHAR(64) NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    reviewer_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    commit_id VARCHAR(64) NOT NULL,
    state VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    body TEXT NOT NULL DEFAULT '',
    dismissed_by_user_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pr_reviews_pr ON pull_request_reviews(pr_id);
CREATE INDEX IF NOT EXISTS idx_pr_reviews_reviewer ON pull_request_reviews(reviewer_id);

-- 17. Pull Request Review Comments (Inline Diff Comments)
CREATE TABLE IF NOT EXISTS pull_request_review_comments (
    id VARCHAR(64) PRIMARY KEY,
    pr_id VARCHAR(64) NOT NULL REFERENCES pull_requests(id) ON DELETE CASCADE,
    review_id VARCHAR(64) REFERENCES pull_request_reviews(id) ON DELETE SET NULL,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    file_path TEXT NOT NULL,
    commit_id VARCHAR(64) NOT NULL,
    diff_side VARCHAR(16) NOT NULL DEFAULT 'RIGHT',
    line_number INTEGER NOT NULL,
    body TEXT NOT NULL,
    is_resolved BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_pr_review_comments_pr ON pull_request_review_comments(pr_id);

-- 18. Branch Protection Rules
CREATE TABLE IF NOT EXISTS branch_protection_rules (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    branch_pattern VARCHAR(255) NOT NULL,
    require_pull_request BOOLEAN NOT NULL DEFAULT TRUE,
    required_approvals INTEGER NOT NULL DEFAULT 1,
    prevent_self_approval BOOLEAN NOT NULL DEFAULT TRUE,
    require_changes_addressed BOOLEAN NOT NULL DEFAULT TRUE,
    dismiss_stale_reviews_on_push BOOLEAN NOT NULL DEFAULT TRUE,
    restrict_merges_to_admins BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, branch_pattern)
);

CREATE INDEX IF NOT EXISTS idx_branch_protection_repo ON branch_protection_rules(repo_id);

-- 19. User Notifications
CREATE TABLE IF NOT EXISTS user_notifications (
    id VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reason VARCHAR(64) NOT NULL,
    resource_type VARCHAR(64) NOT NULL,
    resource_id VARCHAR(64) NOT NULL,
    title VARCHAR(255) NOT NULL,
    message TEXT NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifications_user_read ON user_notifications(user_id, is_read);
CREATE INDEX IF NOT EXISTS idx_notifications_user ON user_notifications(user_id);

-- 20. Repository Activity Events
CREATE TABLE IF NOT EXISTS repository_activity_events (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    actor_id VARCHAR(64) REFERENCES users(id) ON DELETE SET NULL,
    event_type VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_activity_repo ON repository_activity_events(repo_id);
