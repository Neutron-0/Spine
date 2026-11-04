-- 0009_webhooks.sql
-- Webhook subscriptions, transactional outbox, and delivery logs.

CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    target_url TEXT NOT NULL,
    secret_hash TEXT NOT NULL,
    secret_ciphertext TEXT NOT NULL,
    events TEXT[] NOT NULL,
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhooks_owner ON webhook_subscriptions (owner_type, owner_id);

CREATE TABLE IF NOT EXISTS webhook_outbox (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    payload TEXT NOT NULL,
    attempts INT NOT NULL DEFAULT 0,
    max_attempts INT NOT NULL DEFAULT 5,
    status TEXT NOT NULL DEFAULT 'PENDING',
    last_error TEXT,
    created_at_nanos BIGINT NOT NULL,
    next_attempt_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_outbox_status ON webhook_outbox (status, next_attempt_at_nanos);

CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id TEXT PRIMARY KEY,
    subscription_id TEXT NOT NULL REFERENCES webhook_subscriptions (id) ON DELETE CASCADE,
    outbox_id TEXT REFERENCES webhook_outbox (id) ON DELETE SET NULL,
    event_type TEXT NOT NULL,
    target_url TEXT NOT NULL,
    status_code INT,
    duration_ms BIGINT NOT NULL,
    request_headers TEXT NOT NULL,
    request_payload TEXT NOT NULL,
    response_body TEXT,
    error_message TEXT,
    success BOOLEAN NOT NULL,
    delivered_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliveries_sub ON webhook_deliveries (subscription_id, delivered_at_nanos DESC);
