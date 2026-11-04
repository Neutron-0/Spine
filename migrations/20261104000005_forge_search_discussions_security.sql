-- Spine Forge Search, Discussions, Discovery, and Security Center Schema
-- Migration: 20261104000005_forge_search_discussions_security.sql

-- 1. Discussion Categories
CREATE TABLE IF NOT EXISTS discussion_categories (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    name VARCHAR(128) NOT NULL,
    slug VARCHAR(128) NOT NULL,
    description TEXT,
    emoji VARCHAR(32) NOT NULL DEFAULT '💬',
    is_answerable BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_discussion_categories_repo ON discussion_categories(repo_id);

-- 2. Discussions
CREATE TABLE IF NOT EXISTS discussions (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    category_id VARCHAR(64) NOT NULL REFERENCES discussion_categories(id) ON DELETE CASCADE,
    number INTEGER NOT NULL,
    title VARCHAR(256) NOT NULL,
    body TEXT NOT NULL,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    is_pinned BOOLEAN NOT NULL DEFAULT FALSE,
    is_locked BOOLEAN NOT NULL DEFAULT FALSE,
    answered_comment_id VARCHAR(64),
    comments_count INTEGER NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, number)
);

CREATE INDEX IF NOT EXISTS idx_discussions_repo ON discussions(repo_id, created_at_nanos DESC);
CREATE INDEX IF NOT EXISTS idx_discussions_category ON discussions(category_id);

-- 3. Discussion Comments
CREATE TABLE IF NOT EXISTS discussion_comments (
    id VARCHAR(64) PRIMARY KEY,
    discussion_id VARCHAR(64) NOT NULL REFERENCES discussions(id) ON DELETE CASCADE,
    parent_comment_id VARCHAR(64) REFERENCES discussion_comments(id) ON DELETE CASCADE,
    author_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    body TEXT NOT NULL,
    is_answer BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_discussion_comments_disc ON discussion_comments(discussion_id, created_at_nanos ASC);

-- 4. Discussion Reactions
CREATE TABLE IF NOT EXISTS discussion_reactions (
    id VARCHAR(64) PRIMARY KEY,
    subject_type VARCHAR(16) NOT NULL,
    subject_id VARCHAR(64) NOT NULL,
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    reaction VARCHAR(32) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(subject_type, subject_id, user_id, reaction)
);

CREATE INDEX IF NOT EXISTS idx_discussion_reactions_subj ON discussion_reactions(subject_type, subject_id);

-- 5. Discussion Subscriptions
CREATE TABLE IF NOT EXISTS discussion_subscriptions (
    user_id VARCHAR(64) NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    discussion_id VARCHAR(64) NOT NULL REFERENCES discussions(id) ON DELETE CASCADE,
    state VARCHAR(16) NOT NULL DEFAULT 'SUBSCRIBED',
    updated_at_nanos BIGINT NOT NULL,
    PRIMARY KEY(user_id, discussion_id)
);

-- 6. Repository Topics
CREATE TABLE IF NOT EXISTS repository_topics (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    topic VARCHAR(64) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, topic)
);

CREATE INDEX IF NOT EXISTS idx_repo_topics_topic ON repository_topics(topic);

-- 7. Repository Languages
CREATE TABLE IF NOT EXISTS repository_languages (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    language VARCHAR(64) NOT NULL,
    bytes BIGINT NOT NULL,
    percentage DOUBLE PRECISION NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, language)
);

CREATE INDEX IF NOT EXISTS idx_repo_languages_repo ON repository_languages(repo_id);

-- 8. Extended User Profiles
CREATE TABLE IF NOT EXISTS user_profiles (
    user_id VARCHAR(64) PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    bio TEXT,
    company VARCHAR(128),
    location VARCHAR(128),
    website_url VARCHAR(256),
    twitter_username VARCHAR(64),
    profile_readme TEXT,
    pinned_repo_ids JSONB NOT NULL DEFAULT '[]',
    updated_at_nanos BIGINT NOT NULL
);

-- 9. Security Scans
CREATE TABLE IF NOT EXISTS security_scans (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_oid VARCHAR(64) NOT NULL,
    scan_type VARCHAR(32) NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'COMPLETED',
    findings_count INTEGER NOT NULL DEFAULT 0,
    started_at_nanos BIGINT NOT NULL,
    completed_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_security_scans_repo ON security_scans(repo_id, created_at_nanos DESC);

-- 10. Secret Scan Findings
CREATE TABLE IF NOT EXISTS secret_scan_findings (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_oid VARCHAR(64) NOT NULL,
    file_path VARCHAR(512) NOT NULL,
    line_number INTEGER NOT NULL,
    rule_id VARCHAR(64) NOT NULL,
    rule_name VARCHAR(128) NOT NULL,
    severity VARCHAR(16) NOT NULL,
    safe_fingerprint VARCHAR(128) NOT NULL,
    status VARCHAR(16) NOT NULL DEFAULT 'OPEN',
    created_at_nanos BIGINT NOT NULL,
    resolved_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_secret_findings_repo ON secret_scan_findings(repo_id, status);

-- 11. Dependency Inventories
CREATE TABLE IF NOT EXISTS dependency_inventories (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    ecosystem VARCHAR(32) NOT NULL DEFAULT 'CARGO',
    package_name VARCHAR(128) NOT NULL,
    version VARCHAR(64) NOT NULL,
    is_direct BOOLEAN NOT NULL DEFAULT TRUE,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, ecosystem, package_name, version)
);

CREATE INDEX IF NOT EXISTS idx_dep_inventory_repo ON dependency_inventories(repo_id);

-- 12. Security Advisories
CREATE TABLE IF NOT EXISTS security_advisories (
    id VARCHAR(64) PRIMARY KEY,
    advisory_id VARCHAR(64) NOT NULL UNIQUE,
    ecosystem VARCHAR(32) NOT NULL DEFAULT 'CARGO',
    package_name VARCHAR(128) NOT NULL,
    vulnerable_version_range VARCHAR(128) NOT NULL,
    patched_version VARCHAR(64),
    severity VARCHAR(16) NOT NULL,
    title VARCHAR(256) NOT NULL,
    description TEXT NOT NULL,
    published_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_advisories_pkg ON security_advisories(ecosystem, package_name);

-- 13. Dependency Vulnerabilities
CREATE TABLE IF NOT EXISTS dependency_vulnerabilities (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    dependency_id VARCHAR(64) NOT NULL REFERENCES dependency_inventories(id) ON DELETE CASCADE,
    advisory_id VARCHAR(64) NOT NULL,
    package_name VARCHAR(128) NOT NULL,
    affected_version VARCHAR(64) NOT NULL,
    severity VARCHAR(16) NOT NULL,
    patched_version VARCHAR(64),
    status VARCHAR(16) NOT NULL DEFAULT 'OPEN',
    created_at_nanos BIGINT NOT NULL,
    resolved_at_nanos BIGINT,
    UNIQUE(repo_id, dependency_id, advisory_id)
);

CREATE INDEX IF NOT EXISTS idx_dep_vulns_repo ON dependency_vulnerabilities(repo_id, status);
