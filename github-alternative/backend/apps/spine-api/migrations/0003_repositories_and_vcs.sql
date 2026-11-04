-- 0003_repositories_and_vcs.sql
-- Repositories, permissions, deploy keys, branch protection, provisioning jobs, topics, and languages.

CREATE TABLE IF NOT EXISTS forge_repositories (
    id TEXT PRIMARY KEY,
    owner_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    owner_username TEXT NOT NULL,
    owner_org_id TEXT REFERENCES organizations (id) ON DELETE SET NULL,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL,
    description TEXT,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    default_branch TEXT NOT NULL DEFAULT 'main',
    native_repo_id TEXT NOT NULL UNIQUE,
    durability_policy TEXT NOT NULL DEFAULT 'SingleCopy',
    provisioning_status TEXT NOT NULL DEFAULT 'READY',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    CONSTRAINT uq_repo_owner_name UNIQUE (owner_user_id, normalized_name)
);

CREATE INDEX IF NOT EXISTS idx_repos_owner ON forge_repositories (owner_user_id);
CREATE INDEX IF NOT EXISTS idx_repos_org ON forge_repositories (owner_org_id);
CREATE INDEX IF NOT EXISTS idx_repos_native ON forge_repositories (native_repo_id);

CREATE TABLE IF NOT EXISTS repository_permissions (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    permission_role TEXT NOT NULL DEFAULT 'READ',
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_user_perm UNIQUE (repo_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_repo_perm_user ON repository_permissions (user_id);
CREATE INDEX IF NOT EXISTS idx_repo_perm_repo ON repository_permissions (repo_id);

CREATE TABLE IF NOT EXISTS deploy_keys (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    public_key TEXT NOT NULL,
    key_type TEXT NOT NULL,
    write_enabled BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    last_used_at_nanos BIGINT,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_deploy_keys_repo ON deploy_keys (repository_id);
CREATE INDEX IF NOT EXISTS idx_deploy_keys_fp ON deploy_keys (fingerprint);

CREATE TABLE IF NOT EXISTS branch_protection_rules (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    branch_pattern TEXT NOT NULL,
    require_pull_request BOOLEAN NOT NULL DEFAULT FALSE,
    required_approvals INT NOT NULL DEFAULT 1,
    prevent_self_approval BOOLEAN NOT NULL DEFAULT TRUE,
    require_changes_addressed BOOLEAN NOT NULL DEFAULT TRUE,
    dismiss_stale_reviews_on_push BOOLEAN NOT NULL DEFAULT TRUE,
    restrict_merges_to_admins BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_branch_pattern UNIQUE (repo_id, branch_pattern)
);

CREATE INDEX IF NOT EXISTS idx_branch_rules_repo ON branch_protection_rules (repo_id);

CREATE TABLE IF NOT EXISTS repository_provisioning_jobs (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    status TEXT NOT NULL DEFAULT 'PENDING',
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    idempotency_key TEXT NOT NULL UNIQUE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_prov_jobs_status ON repository_provisioning_jobs (status);

CREATE TABLE IF NOT EXISTS repository_topics (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    topic TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_topic UNIQUE (repo_id, topic)
);

CREATE INDEX IF NOT EXISTS idx_topics_repo ON repository_topics (repo_id);
CREATE INDEX IF NOT EXISTS idx_topics_topic ON repository_topics (topic);

CREATE TABLE IF NOT EXISTS repository_languages (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    language TEXT NOT NULL,
    bytes BIGINT NOT NULL,
    percentage DOUBLE PRECISION NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_lang UNIQUE (repo_id, language)
);

CREATE INDEX IF NOT EXISTS idx_lang_repo ON repository_languages (repo_id);
