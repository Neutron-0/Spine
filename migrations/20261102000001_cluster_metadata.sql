-- Spine Cluster Metadata Schema (Batch 07)
-- Production PostgreSQL DDL for Spine Forge Distributed Storage Control Plane

CREATE TABLE IF NOT EXISTS storage_nodes (
    node_id VARCHAR(64) PRIMARY KEY,
    public_key_hex VARCHAR(64) NOT NULL UNIQUE,
    fingerprint VARCHAR(64) NOT NULL UNIQUE,
    status VARCHAR(32) NOT NULL DEFAULT 'JOINING',
    listen_addr VARCHAR(128) NOT NULL,
    storage_root VARCHAR(512) NOT NULL,
    failure_domain_region VARCHAR(64) NOT NULL DEFAULT 'default',
    failure_domain_zone VARCHAR(64) NOT NULL DEFAULT 'default',
    failure_domain_host VARCHAR(128) NOT NULL DEFAULT 'localhost',
    storage_class VARCHAR(64) NOT NULL DEFAULT 'standard',
    total_capacity_bytes BIGINT NOT NULL DEFAULT 0,
    used_capacity_bytes BIGINT NOT NULL DEFAULT 0,
    reserved_capacity_bytes BIGINT NOT NULL DEFAULT 0,
    enrolled_at_nanos BIGINT NOT NULL,
    last_heartbeat_nanos BIGINT NOT NULL,
    active_tasks INT NOT NULL DEFAULT 0,
    error_count INT NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS node_identities (
    public_key_hex VARCHAR(64) PRIMARY KEY,
    node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id) ON DELETE CASCADE,
    enrollment_token VARCHAR(128) NOT NULL,
    registered_at_nanos BIGINT NOT NULL,
    revoked BOOLEAN NOT NULL DEFAULT FALSE,
    revoked_at_nanos BIGINT
);

CREATE TABLE IF NOT EXISTS node_heartbeats (
    id BIGSERIAL PRIMARY KEY,
    node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id) ON DELETE CASCADE,
    timestamp_nanos BIGINT NOT NULL,
    node_generation BIGINT NOT NULL,
    total_bytes BIGINT NOT NULL,
    used_bytes BIGINT NOT NULL,
    available_bytes BIGINT NOT NULL,
    reserved_bytes BIGINT NOT NULL,
    active_repos INT NOT NULL,
    active_replication_tasks INT NOT NULL,
    error_counter INT NOT NULL,
    status VARCHAR(32) NOT NULL,
    protocol_version VARCHAR(32) NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_node_heartbeats_node_ts ON node_heartbeats(node_id, timestamp_nanos DESC);

CREATE TABLE IF NOT EXISTS repositories (
    repo_id VARCHAR(128) PRIMARY KEY,
    name VARCHAR(256) NOT NULL,
    owner VARCHAR(128) NOT NULL,
    durability_policy VARCHAR(32) NOT NULL DEFAULT 'THREE_COPIES',
    committed_generation BIGINT NOT NULL DEFAULT 0,
    current_ownership_epoch BIGINT NOT NULL DEFAULT 1,
    storage_quota_bytes BIGINT NOT NULL DEFAULT 10737418240, -- 10 GB default
    warning_quota_bytes BIGINT NOT NULL DEFAULT 8589934592,  -- 8 GB default
    state VARCHAR(32) NOT NULL DEFAULT 'HEALTHY',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS repository_placements (
    repo_id VARCHAR(128) PRIMARY KEY REFERENCES repositories(repo_id) ON DELETE CASCADE,
    primary_node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    placement_generation BIGINT NOT NULL DEFAULT 1,
    required_copies INT NOT NULL DEFAULT 3,
    state VARCHAR(32) NOT NULL DEFAULT 'HEALTHY',
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS repository_replicas (
    id BIGSERIAL PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    is_primary BOOLEAN NOT NULL DEFAULT FALSE,
    last_synchronized_generation BIGINT NOT NULL DEFAULT 0,
    verified BOOLEAN NOT NULL DEFAULT FALSE,
    state VARCHAR(32) NOT NULL DEFAULT 'SYNCED',
    last_verified_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, node_id)
);

CREATE TABLE IF NOT EXISTS replication_transactions (
    transaction_id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    previous_generation BIGINT NOT NULL,
    proposed_generation BIGINT NOT NULL,
    primary_node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    ownership_epoch BIGINT NOT NULL,
    phase VARCHAR(32) NOT NULL DEFAULT 'INITIATED',
    required_durability INT NOT NULL,
    participating_nodes JSONB NOT NULL,
    prepared_nodes JSONB NOT NULL DEFAULT '[]',
    commit_decided_at_nanos BIGINT,
    completed_at_nanos BIGINT,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS committed_generations (
    id BIGSERIAL PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    generation BIGINT NOT NULL,
    ownership_epoch BIGINT NOT NULL,
    commit_transaction_id VARCHAR(64) NOT NULL,
    root_pack_id VARCHAR(64) NOT NULL,
    catalog_checksum VARCHAR(64) NOT NULL,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, generation)
);

CREATE TABLE IF NOT EXISTS ownership_epochs (
    id BIGSERIAL PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    epoch BIGINT NOT NULL,
    primary_node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    granted_at_nanos BIGINT NOT NULL,
    fenced BOOLEAN NOT NULL DEFAULT FALSE,
    fenced_at_nanos BIGINT,
    UNIQUE(repo_id, epoch)
);

CREATE TABLE IF NOT EXISTS failure_events (
    id BIGSERIAL PRIMARY KEY,
    node_id VARCHAR(64) REFERENCES storage_nodes(node_id),
    repo_id VARCHAR(128) REFERENCES repositories(repo_id),
    event_type VARCHAR(64) NOT NULL,
    severity VARCHAR(32) NOT NULL DEFAULT 'WARNING',
    details JSONB NOT NULL,
    timestamp_nanos BIGINT NOT NULL,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_at_nanos BIGINT
);

CREATE TABLE IF NOT EXISTS repair_jobs (
    job_id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    target_node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    source_node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    missing_packs JSONB NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'QUEUED',
    bytes_transferred BIGINT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    completed_at_nanos BIGINT,
    error_message TEXT
);

CREATE TABLE IF NOT EXISTS storage_tombstones (
    tombstone_id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    object_id VARCHAR(64) NOT NULL,
    object_type VARCHAR(32) NOT NULL,
    pack_id VARCHAR(64),
    authorization_generation BIGINT NOT NULL,
    status VARCHAR(32) NOT NULL DEFAULT 'PENDING',
    created_at_nanos BIGINT NOT NULL,
    retention_expires_at_nanos BIGINT NOT NULL,
    deleted_at_nanos BIGINT
);

CREATE TABLE IF NOT EXISTS integrity_reports (
    report_id VARCHAR(64) PRIMARY KEY,
    repo_id VARCHAR(128) NOT NULL REFERENCES repositories(repo_id) ON DELETE CASCADE,
    node_id VARCHAR(64) NOT NULL REFERENCES storage_nodes(node_id),
    generation BIGINT NOT NULL,
    objects_checked BIGINT NOT NULL,
    packs_checked INT NOT NULL,
    corrupt_objects INT NOT NULL DEFAULT 0,
    missing_objects INT NOT NULL DEFAULT 0,
    is_healthy BOOLEAN NOT NULL,
    report_json JSONB NOT NULL,
    timestamp_nanos BIGINT NOT NULL
);
