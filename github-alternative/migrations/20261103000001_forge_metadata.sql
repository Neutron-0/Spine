-- Spine Forge Metadata Schema (Batch 08)
-- Production PostgreSQL DDL for Spine Forge Identity, Authentication, Permissions, and Repositories

-- 1. Users table
CREATE TABLE IF NOT EXISTS users (
    id VARCHAR(64) PRIMARY KEY,
    username VARCHAR(64) NOT NULL UNIQUE,
    normalized_username VARCHAR(64) NOT NULL UNIQUE,
    email VARCHAR(256) NOT NULL UNIQUE,
    normalized_email VARCHAR(256) NOT NULL UNIQUE,
    display_name VARCHAR(128) NOT NULL,
    password_hash VARCHAR(256) NOT NULL,
    account_status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE',
    avatar_url VARCHAR(512),
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    last_login_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_users_norm_username ON users(normalized_username);
CREATE INDEX IF NOT EXISTS idx_users_norm_email ON users(normalized_email);

-- 2. User sessions table (stores session token hashes only)
CREATE TABLE IF NOT EXISTS user_sessions (
    id VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    session_token_hash VARCHAR(64) NOT NULL UNIQUE,
    ip_address VARCHAR(64),
    user_agent VARCHAR(256),
    created_at_nanos BIGINT NOT NULL,
    expires_at_nanos BIGINT NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_user_sessions_hash ON user_sessions(session_token_hash);
CREATE INDEX IF NOT EXISTS idx_user_sessions_user_id ON user_sessions(user_id);
CREATE INDEX IF NOT EXISTS idx_user_sessions_expires ON user_sessions(expires_at_nanos);

-- 3. Email verification tokens
CREATE TABLE IF NOT EXISTS email_verification_tokens (
    token_hash VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at_nanos BIGINT NOT NULL,
    used_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_email_verify_user ON email_verification_tokens(user_id);

-- 4. Password reset tokens
CREATE TABLE IF NOT EXISTS password_reset_tokens (
    token_hash VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    expires_at_nanos BIGINT NOT NULL,
    used_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_password_reset_user ON password_reset_tokens(user_id);

-- 5. Personal access tokens (PATs)
CREATE TABLE IF NOT EXISTS personal_access_tokens (
    id VARCHAR(64) PRIMARY KEY,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name VARCHAR(128) NOT NULL,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    scopes JSONB NOT NULL DEFAULT '["repo:read", "repo:write"]',
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    last_used_at_nanos BIGINT,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_pats_token_hash ON personal_access_tokens(token_hash);
CREATE INDEX IF NOT EXISTS idx_pats_user_id ON personal_access_tokens(user_id);

-- 6. Forge repositories table
CREATE TABLE IF NOT EXISTS forge_repositories (
    id VARCHAR(64) PRIMARY KEY,
    owner_user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
    owner_username VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    normalized_name VARCHAR(128) NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    default_branch VARCHAR(64) NOT NULL DEFAULT 'main',
    native_repo_id VARCHAR(128) NOT NULL UNIQUE,
    durability_policy VARCHAR(32) NOT NULL DEFAULT 'THREE_COPIES',
    provisioning_status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    archived BOOLEAN NOT NULL DEFAULT FALSE,
    UNIQUE(owner_user_id, normalized_name)
);

CREATE INDEX IF NOT EXISTS idx_forge_repos_owner ON forge_repositories(owner_username, normalized_name);
CREATE INDEX IF NOT EXISTS idx_forge_repos_native_id ON forge_repositories(native_repo_id);
CREATE INDEX IF NOT EXISTS idx_forge_repos_status ON forge_repositories(provisioning_status);

-- 7. Repository permissions table
CREATE TABLE IF NOT EXISTS repository_permissions (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permission_role VARCHAR(32) NOT NULL DEFAULT 'COLLABORATOR',
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_repo_perm_repo_user ON repository_permissions(repo_id, user_id);

-- 8. Repository provisioning jobs table (outbox pattern)
CREATE TABLE IF NOT EXISTS repository_provisioning_jobs (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES forge_repositories(id) ON DELETE CASCADE,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    attempts INT NOT NULL DEFAULT 0,
    last_error TEXT,
    idempotency_key VARCHAR(128) NOT NULL UNIQUE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_provisioning_jobs_status ON repository_provisioning_jobs(status, created_at_nanos ASC);

-- 9. Audit events table
CREATE TABLE IF NOT EXISTS audit_events (
    id BIGSERIAL PRIMARY KEY,
    user_id VARCHAR(64),
    action VARCHAR(64) NOT NULL,
    resource_type VARCHAR(64) NOT NULL,
    resource_id VARCHAR(128) NOT NULL,
    ip_address VARCHAR(64),
    metadata JSONB NOT NULL DEFAULT '{}',
    timestamp_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_events_ts ON audit_events(timestamp_nanos DESC);
CREATE INDEX IF NOT EXISTS idx_audit_events_resource ON audit_events(resource_type, resource_id);
