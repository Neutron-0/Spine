-- 0001_core_identity.sql
-- Core users, sessions, profiles, PATs, SSH keys, password resets, email verification, and audit logs.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    username TEXT NOT NULL,
    normalized_username TEXT NOT NULL UNIQUE,
    email TEXT NOT NULL,
    normalized_email TEXT NOT NULL,
    display_name TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    account_status TEXT NOT NULL DEFAULT 'ACTIVE',
    avatar_url TEXT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    last_login_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_users_email ON users (normalized_email);

CREATE TABLE IF NOT EXISTS user_profiles (
    user_id TEXT PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    bio TEXT,
    location TEXT,
    website TEXT,
    company TEXT,
    social_links_json TEXT,
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS user_sessions (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    session_token TEXT NOT NULL UNIQUE,
    csrf_token TEXT NOT NULL,
    user_agent TEXT,
    ip_address TEXT,
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    last_active_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sessions_user_id ON user_sessions (user_id);
CREATE INDEX IF NOT EXISTS idx_sessions_token ON user_sessions (session_token);

CREATE TABLE IF NOT EXISTS personal_access_tokens (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    scopes_json TEXT NOT NULL,
    expires_at_nanos BIGINT NOT NULL,
    last_used_at_nanos BIGINT,
    created_at_nanos BIGINT NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_pat_user_id ON personal_access_tokens (user_id);
CREATE INDEX IF NOT EXISTS idx_pat_token_hash ON personal_access_tokens (token_hash);

CREATE TABLE IF NOT EXISTS user_ssh_keys (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    public_key TEXT NOT NULL,
    key_type TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    last_used_at_nanos BIGINT,
    revoked BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE INDEX IF NOT EXISTS idx_ssh_keys_user_id ON user_ssh_keys (user_id);
CREATE INDEX IF NOT EXISTS idx_ssh_keys_fingerprint ON user_ssh_keys (fingerprint);

CREATE TABLE IF NOT EXISTS password_reset_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    used BOOLEAN NOT NULL DEFAULT FALSE
);

CREATE TABLE IF NOT EXISTS email_verification_tokens (
    token_hash TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    new_email TEXT NOT NULL,
    expires_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS audit_events (
    id TEXT PRIMARY KEY,
    actor_user_id TEXT,
    event_type TEXT NOT NULL,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    metadata_json TEXT,
    ip_address TEXT,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_audit_created ON audit_events (created_at_nanos DESC);
