-- 0008_workflows_and_runners.sql
-- Workflows, runs, jobs, steps, logs, secrets, check results, and runner registrations.

CREATE TABLE IF NOT EXISTS workflow_definitions (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    name TEXT NOT NULL,
    yaml_content TEXT NOT NULL,
    digest TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_workflow_repo_path UNIQUE (repo_id, path)
);

CREATE INDEX IF NOT EXISTS idx_workflow_defs_repo ON workflow_definitions (repo_id);

CREATE TABLE IF NOT EXISTS workflow_runs (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    workflow_id TEXT NOT NULL,
    workflow_name TEXT NOT NULL,
    commit_oid TEXT NOT NULL,
    ref_name TEXT NOT NULL,
    trigger_event TEXT NOT NULL,
    actor_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    status TEXT NOT NULL,
    conclusion TEXT,
    definition_yaml TEXT NOT NULL,
    definition_digest TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_runs_repo ON workflow_runs (repo_id, created_at_nanos DESC);

CREATE TABLE IF NOT EXISTS workflow_jobs (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs (id) ON DELETE CASCADE,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    job_key TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    runner_id TEXT,
    runs_on JSONB NOT NULL DEFAULT '[]'::jsonb,
    needs JSONB NOT NULL DEFAULT '[]'::jsonb,
    attempt INT NOT NULL DEFAULT 1,
    fencing_token BIGINT NOT NULL DEFAULT 0,
    lease_expires_at_nanos BIGINT,
    exit_code INT,
    created_at_nanos BIGINT NOT NULL,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_workflow_jobs_run ON workflow_jobs (run_id);
CREATE INDEX IF NOT EXISTS idx_workflow_jobs_status ON workflow_jobs (status);

CREATE TABLE IF NOT EXISTS workflow_steps (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES workflow_jobs (id) ON DELETE CASCADE,
    step_index INT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL,
    exit_code INT,
    started_at_nanos BIGINT,
    completed_at_nanos BIGINT,
    CONSTRAINT uq_step_job_index UNIQUE (job_id, step_index)
);

CREATE TABLE IF NOT EXISTS workflow_logs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES workflow_jobs (id) ON DELETE CASCADE,
    step_index INT,
    chunk_index INT NOT NULL,
    content TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_workflow_logs_job ON workflow_logs (job_id, chunk_index);

CREATE TABLE IF NOT EXISTS workflow_secrets (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    name TEXT NOT NULL,
    encrypted_value TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_workflow_secret UNIQUE (owner_type, owner_id, name)
);

CREATE TABLE IF NOT EXISTS workflow_check_results (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    commit_oid TEXT NOT NULL,
    check_name TEXT NOT NULL,
    status TEXT NOT NULL,
    conclusion TEXT,
    details_url TEXT,
    run_id TEXT,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_check_commit_name UNIQUE (repo_id, commit_oid, check_name)
);

CREATE INDEX IF NOT EXISTS idx_checks_repo_commit ON workflow_check_results (repo_id, commit_oid);

CREATE TABLE IF NOT EXISTS runner_registrations (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    hostname TEXT NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    os TEXT NOT NULL,
    arch TEXT NOT NULL,
    labels JSONB NOT NULL DEFAULT '[]'::jsonb,
    max_concurrency INT NOT NULL DEFAULT 1,
    active_jobs INT NOT NULL DEFAULT 0,
    status TEXT NOT NULL DEFAULT 'ONLINE',
    last_heartbeat_at_nanos BIGINT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_runners_status ON runner_registrations (status, last_heartbeat_at_nanos);
