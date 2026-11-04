-- 0002_organizations_and_teams.sql
-- Organizations, memberships, invitations, teams, team memberships, and team permissions.

CREATE TABLE IF NOT EXISTS organizations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    display_name TEXT NOT NULL,
    description TEXT,
    avatar_url TEXT,
    owner_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_orgs_name ON organizations (normalized_name);

CREATE TABLE IF NOT EXISTS organization_memberships (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    role TEXT NOT NULL DEFAULT 'MEMBER',
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_org_user UNIQUE (org_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_org_mem_user ON organization_memberships (user_id);
CREATE INDEX IF NOT EXISTS idx_org_mem_org ON organization_memberships (org_id);

CREATE TABLE IF NOT EXISTS organization_invitations (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    role TEXT NOT NULL DEFAULT 'MEMBER',
    invited_by_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE,
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_org_inv_email ON organization_invitations (email);

CREATE TABLE IF NOT EXISTS teams (
    id TEXT PRIMARY KEY,
    org_id TEXT NOT NULL REFERENCES organizations (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    normalized_slug TEXT NOT NULL,
    description TEXT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_team_org_slug UNIQUE (org_id, normalized_slug)
);

CREATE INDEX IF NOT EXISTS idx_teams_org ON teams (org_id);

CREATE TABLE IF NOT EXISTS team_memberships (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    is_maintainer BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_team_user UNIQUE (team_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_team_mem_user ON team_memberships (user_id);
CREATE INDEX IF NOT EXISTS idx_team_mem_team ON team_memberships (team_id);

CREATE TABLE IF NOT EXISTS team_repository_permissions (
    id TEXT PRIMARY KEY,
    team_id TEXT NOT NULL REFERENCES teams (id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL,
    permission_level TEXT NOT NULL DEFAULT 'READ',
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_team_repo UNIQUE (team_id, repo_id)
);

CREATE INDEX IF NOT EXISTS idx_team_repo_perm_repo ON team_repository_permissions (repo_id);
