-- Spine Forge Automation, Webhooks, Workflows, and Runners Schema
-- Migration: 20261104000004_forge_automation_webhooks_workflows.sql

-- 1. Webhook Subscriptions
CREATE TABLE IF NOT EXISTS webhook_subscriptions (
    id VARCHAR(64) PRIMARY KEY,
    owner_type VARCHAR(16) NOT NULL, -- 'REPO' or 'ORG'
    owner_id VARCHAR(64) NOT NULL,
    target_url VARCHAR(1024) NOT NULL,
    secret_hash VARCHAR(64) NOT NULL,
    secret_ciphertext VARCHAR(512) NOT NULL,
    events VARCHAR(512) NOT NULL, -- e.g. "push,pull_request,release"
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_subs_owner ON webhook_subscriptions(owner_type, owner_id);

-- 2. Webhook Outbox (Transactional Outbox Pattern)
CREATE TABLE IF NOT EXISTS webhook_outbox (
    id VARCHAR(64) PRIMARY KEY,
    event_type VARCHAR(64) NOT NULL,
    owner_type VARCHAR(16) NOT NULL,
    owner_id VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    status VARCHAR(16) NOT NULL DEFAULT 'PENDING', -- 'PENDING', 'PROCESSING', 'DELIVERED', 'FAILED'
    last_error TEXT,
    created_at_nanos BIGINT NOT NULL,
    next_attempt_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_outbox_pending ON webhook_outbox(status, next_attempt_at_nanos);

-- 3. Webhook Delivery History
CREATE TABLE IF NOT EXISTS webhook_deliveries (
    id VARCHAR(64) PRIMARY KEY,
    subscription_id VARCHAR(64) NOT NULL REFERENCES webhook_subscriptions(id) ON DELETE CASCADE,
    event_type VARCHAR(64) NOT NULL,
    payload JSONB NOT NULL,
    response_status INTEGER,
    response_body TEXT,
    execution_duration_ms BIGINT NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,
    status VARCHAR(16) NOT NULL, -- 'SUCCESS', 'FAILED'
    delivered_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_webhook_deliv_sub ON webhook_deliveries(subscription_id, delivered_at_nanos DESC);

-- 4. Workflow Definitions (Repository-tracked workflows)
CREATE TABLE IF NOT EXISTS workflow_definitions (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    path VARCHAR(256) NOT NULL,
    name VARCHAR(128) NOT NULL,
    yaml_content TEXT NOT NULL,
    digest VARCHAR(64) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, path)
);

-- 5. Workflow Runs (Execution instances bound to commits)
CREATE TABLE IF NOT EXISTS workflow_runs (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    workflow_id VARCHAR(64) NOT NULL,
    workflow_name VARCHAR(128) NOT NULL,
    commit_oid VARCHAR(64) NOT NULL,
    ref_name VARCHAR(128) NOT NULL,
    trigger_event VARCHAR(64) NOT NULL,
    actor_id VARCHAR(64) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'QUEUED', -- 'QUEUED', 'RUNNING', 'SUCCEEDED', 'FAILED', 'CANCELLED'
    conclusion VARCHAR(32),
    definition_yaml TEXT NOT NULL,
    definition_digest VARCHAR(64) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_repo ON workflow_runs(repo_id, created_at_nanos DESC);
CREATE INDEX IF NOT EXISTS idx_workflow_runs_commit ON workflow_runs(repo_id, commit_oid);

-- 6. Workflow Jobs (DAG nodes)
CREATE TABLE IF NOT EXISTS workflow_jobs (
    id VARCHAR(64) PRIMARY KEY,
    run_id VARCHAR(64) NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    job_key VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'QUEUED',
    runner_id VARCHAR(64),
    runs_on JSONB NOT NULL,
    needs JSONB NOT NULL,
    attempt INTEGER NOT NULL DEFAULT 1,
    fencing_token BIGINT NOT NULL DEFAULT 1,
    lease_expires_at_nanos BIGINT,
    exit_code INTEGER,
    created_at_nanos BIGINT NOT NULL,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_jobs_run ON workflow_jobs(run_id);
CREATE INDEX IF NOT EXISTS idx_workflow_jobs_status ON workflow_jobs(status, lease_expires_at_nanos);

-- 7. Workflow Steps
CREATE TABLE IF NOT EXISTS workflow_steps (
    id VARCHAR(64) PRIMARY KEY,
    job_id VARCHAR(64) NOT NULL REFERENCES workflow_jobs(id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    name VARCHAR(128) NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    exit_code INTEGER,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_steps_job ON workflow_steps(job_id, step_index);

-- 8. Workflow Logs
CREATE TABLE IF NOT EXISTS workflow_logs (
    id VARCHAR(64) PRIMARY KEY,
    job_id VARCHAR(64) NOT NULL REFERENCES workflow_jobs(id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    stream VARCHAR(16) NOT NULL, -- 'stdout', 'stderr'
    content TEXT NOT NULL,
    timestamp_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workflow_logs_job ON workflow_logs(job_id, step_index, timestamp_nanos);

-- 9. Runner Registrations
CREATE TABLE IF NOT EXISTS runner_registrations (
    id VARCHAR(64) PRIMARY KEY,
    name VARCHAR(128) NOT NULL,
    hostname VARCHAR(128) NOT NULL,
    token_hash VARCHAR(64) NOT NULL UNIQUE,
    os VARCHAR(32) NOT NULL,
    arch VARCHAR(32) NOT NULL,
    labels JSONB NOT NULL,
    max_concurrency INTEGER NOT NULL DEFAULT 1,
    active_jobs INTEGER NOT NULL DEFAULT 0,
    status VARCHAR(32) NOT NULL DEFAULT 'ACTIVE', -- 'ACTIVE', 'DRAINED', 'REVOKED'
    last_heartbeat_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runners_status ON runner_registrations(status, last_heartbeat_at_nanos);

-- 10. Workflow Secrets
CREATE TABLE IF NOT EXISTS workflow_secrets (
    id VARCHAR(64) PRIMARY KEY,
    owner_type VARCHAR(16) NOT NULL, -- 'REPO' or 'ORG'
    owner_id VARCHAR(64) NOT NULL,
    name VARCHAR(128) NOT NULL,
    ciphertext VARCHAR(1024) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (owner_type, owner_id, name)
);

-- 11. Pull Request / Commit Check Results
CREATE TABLE IF NOT EXISTS workflow_check_results (
    id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(64) NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    commit_oid VARCHAR(64) NOT NULL,
    check_name VARCHAR(128) NOT NULL,
    status VARCHAR(32) NOT NULL, -- 'PENDING', 'SUCCESS', 'FAILURE', 'ERROR'
    run_id VARCHAR(64) NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE (repo_id, commit_oid, check_name)
);

CREATE INDEX IF NOT EXISTS idx_checks_commit ON workflow_check_results(repo_id, commit_oid);
