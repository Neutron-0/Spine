-- 0005_notifications_and_events.sql
-- User notifications and repository activity event feeds.

CREATE TABLE IF NOT EXISTS user_notifications (
    id TEXT PRIMARY KEY,
    user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    reason TEXT NOT NULL,
    resource_type TEXT NOT NULL,
    resource_id TEXT NOT NULL,
    title TEXT NOT NULL,
    message TEXT NOT NULL,
    is_read BOOLEAN NOT NULL DEFAULT FALSE,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_notifs_user ON user_notifications (user_id, is_read, created_at_nanos DESC);

CREATE TABLE IF NOT EXISTS repository_activity_events (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    actor_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_activity_repo ON repository_activity_events (repo_id, created_at_nanos DESC);
CREATE INDEX IF NOT EXISTS idx_activity_actor ON repository_activity_events (actor_id, created_at_nanos DESC);
