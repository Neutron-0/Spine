-- 0006_artifacts_and_lfs.sql
-- Artifact quotas, namespaces, manifests, versions, upload sessions, and Git LFS locks.

CREATE TABLE IF NOT EXISTS artifact_quotas (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL UNIQUE,
    logical_quota_bytes BIGINT NOT NULL,
    used_logical_bytes BIGINT NOT NULL DEFAULT 0,
    used_physical_bytes BIGINT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifact_upload_sessions (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'CREATED',
    total_size BIGINT NOT NULL,
    chunk_size BIGINT NOT NULL,
    expected_chunks INT NOT NULL,
    durability_policy TEXT NOT NULL DEFAULT 'SingleCopy',
    created_at_nanos BIGINT NOT NULL,
    expires_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_art_sessions_expiry ON artifact_upload_sessions (expires_at_nanos);

CREATE TABLE IF NOT EXISTS artifact_manifests (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL,
    whole_object_hash TEXT NOT NULL,
    total_size BIGINT NOT NULL,
    chunk_size BIGINT NOT NULL,
    chunk_count INT NOT NULL,
    content_type TEXT,
    manifest_json TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_art_manifest_hash ON artifact_manifests (whole_object_hash);
CREATE INDEX IF NOT EXISTS idx_art_manifest_artifact_id ON artifact_manifests (artifact_id);

CREATE TABLE IF NOT EXISTS artifact_namespaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    description TEXT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_ns_owner_name UNIQUE (owner_id, name)
);

CREATE INDEX IF NOT EXISTS idx_art_ns_owner ON artifact_namespaces (owner_id);

CREATE TABLE IF NOT EXISTS artifact_versions (
    id TEXT PRIMARY KEY,
    namespace_id TEXT NOT NULL REFERENCES artifact_namespaces (id) ON DELETE CASCADE,
    artifact_name TEXT NOT NULL,
    version TEXT NOT NULL,
    manifest_id TEXT NOT NULL REFERENCES artifact_manifests (id) ON DELETE RESTRICT,
    whole_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    created_by_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_art_ns_name_ver UNIQUE (namespace_id, artifact_name, version)
);

CREATE INDEX IF NOT EXISTS idx_art_ver_ns ON artifact_versions (namespace_id);

CREATE TABLE IF NOT EXISTS lfs_locks (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    owner_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    owner_username TEXT NOT NULL,
    locked_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_lfs_path UNIQUE (repository_id, path)
);

CREATE INDEX IF NOT EXISTS idx_lfs_locks_repo ON lfs_locks (repository_id);
CREATE INDEX IF NOT EXISTS idx_lfs_locks_owner ON lfs_locks (owner_id);

CREATE TABLE IF NOT EXISTS lfs_lock_audits (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    actor_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    action TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_lfs_audits_repo ON lfs_lock_audits (repository_id, created_at_nanos DESC);

CREATE TABLE IF NOT EXISTS lfs_objects (
    id TEXT PRIMARY KEY,
    repository_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    oid TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    manifest_id TEXT NOT NULL REFERENCES artifact_manifests (id) ON DELETE RESTRICT,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_lfs_oid UNIQUE (repository_id, oid)
);

CREATE INDEX IF NOT EXISTS idx_lfs_objects_repo_oid ON lfs_objects (repository_id, oid);
CREATE INDEX IF NOT EXISTS idx_lfs_objects_manifest ON lfs_objects (manifest_id);
CREATE INDEX IF NOT EXISTS idx_lfs_objects_oid ON lfs_objects (oid);

