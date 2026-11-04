-- 0010_discussions.sql
-- Discussion categories, discussions, threaded comments, reactions, and subscriptions.

CREATE TABLE IF NOT EXISTS discussion_categories (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    slug TEXT NOT NULL,
    description TEXT,
    emoji TEXT,
    is_answerable BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_disc_cat_slug UNIQUE (repo_id, slug)
);

CREATE INDEX IF NOT EXISTS idx_disc_cats_repo ON discussion_categories (repo_id);

CREATE TABLE IF NOT EXISTS discussions (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    category_id TEXT NOT NULL REFERENCES discussion_categories (id) ON DELETE RESTRICT,
    number BIGINT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    is_pinned BOOLEAN NOT NULL DEFAULT FALSE,
    is_locked BOOLEAN NOT NULL DEFAULT FALSE,
    answered_comment_id TEXT,
    comments_count INT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_disc_repo_num UNIQUE (repo_id, number)
);

CREATE INDEX IF NOT EXISTS idx_discussions_repo ON discussions (repo_id, created_at_nanos DESC);

CREATE TABLE IF NOT EXISTS discussion_comments (
    id TEXT PRIMARY KEY,
    discussion_id TEXT NOT NULL REFERENCES discussions (id) ON DELETE CASCADE,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    body TEXT NOT NULL,
    parent_comment_id TEXT REFERENCES discussion_comments (id) ON DELETE CASCADE,
    is_answer BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_disc_comments_disc ON discussion_comments (discussion_id);

CREATE TABLE IF NOT EXISTS discussion_reactions (
    id TEXT PRIMARY KEY,
    target_type TEXT NOT NULL,
    target_id TEXT NOT NULL,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    content TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_reaction UNIQUE (target_type, target_id, user_id, content)
);

CREATE TABLE IF NOT EXISTS discussion_subscriptions (
    id TEXT PRIMARY KEY,
    discussion_id TEXT NOT NULL REFERENCES discussions (id) ON DELETE CASCADE,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_disc_sub UNIQUE (discussion_id, user_id)
);
