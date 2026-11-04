-- Batch 11: Spine Forge Distributed Large Object Storage, Spine LFS, Releases, Artifacts, and Package Registries

-- 1. Artifact Quotas & Accounting
CREATE TABLE IF NOT EXISTS artifact_quotas (
    id TEXT PRIMARY KEY,
    owner_type TEXT NOT NULL, -- 'user' or 'organization'
    owner_id TEXT NOT NULL UNIQUE,
    logical_quota_bytes BIGINT NOT NULL DEFAULT 53687091200, -- 50 GB default
    used_logical_bytes BIGINT NOT NULL DEFAULT 0,
    used_physical_bytes BIGINT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

-- 2. Artifact Upload Sessions (Resumable Transfer State Machine)
CREATE TABLE IF NOT EXISTS artifact_upload_sessions (
    id TEXT PRIMARY KEY,
    artifact_id TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    state TEXT NOT NULL, -- 'Created', 'Uploading', 'Verifying', 'Replicating', 'Finalizing', 'Complete', 'Aborted', 'Expired'
    total_size BIGINT NOT NULL,
    chunk_size INT NOT NULL,
    expected_chunks INT NOT NULL,
    durability_policy TEXT NOT NULL, -- 'SingleCopy', 'TwoCopies', 'ThreeCopies'
    created_at_nanos BIGINT NOT NULL,
    expires_at_nanos BIGINT NOT NULL
);

-- 3. Artifact Manifests
CREATE TABLE IF NOT EXISTS artifact_manifests (
    id TEXT PRIMARY KEY, -- manifest_oid
    artifact_id TEXT NOT NULL,
    whole_object_hash TEXT NOT NULL,
    total_size BIGINT NOT NULL,
    chunk_size INT NOT NULL,
    chunk_count INT NOT NULL,
    content_type TEXT,
    manifest_json TEXT NOT NULL,
    created_at_nanos BIGINT NOT NULL
);

-- 4. Repository Releases
CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES repositories(id) ON DELETE CASCADE,
    tag_name TEXT NOT NULL,
    target_commit_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    is_prerelease BOOLEAN NOT NULL DEFAULT FALSE,
    author_id TEXT NOT NULL REFERENCES users(id),
    created_at_nanos BIGINT NOT NULL,
    published_at_nanos BIGINT,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE(repo_id, tag_name)
);

CREATE INDEX IF NOT EXISTS idx_releases_repo_id ON releases(repo_id);

-- 5. Release Assets
CREATE TABLE IF NOT EXISTS release_assets (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL REFERENCES releases(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    manifest_id TEXT NOT NULL,
    whole_hash TEXT NOT NULL,
    downloads_count INT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(release_id, name)
);

CREATE INDEX IF NOT EXISTS idx_release_assets_release ON release_assets(release_id);

-- 6. Generic Artifact Namespaces & Versions
CREATE TABLE IF NOT EXISTS artifact_namespaces (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    description TEXT NOT NULL DEFAULT '',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifact_versions (
    id TEXT PRIMARY KEY,
    namespace_id TEXT NOT NULL REFERENCES artifact_namespaces(id) ON DELETE CASCADE,
    artifact_name TEXT NOT NULL,
    version TEXT NOT NULL,
    manifest_id TEXT NOT NULL,
    whole_hash TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    created_by_user_id TEXT NOT NULL REFERENCES users(id),
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(namespace_id, artifact_name, version)
);

CREATE INDEX IF NOT EXISTS idx_artifact_versions_ns ON artifact_versions(namespace_id, artifact_name);

-- 7. Package Registries & Versions (Cargo and multi-ecosystem)
CREATE TABLE IF NOT EXISTS packages (
    id TEXT PRIMARY KEY,
    ecosystem TEXT NOT NULL DEFAULT 'cargo',
    name TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    description TEXT NOT NULL DEFAULT '',
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    UNIQUE(ecosystem, name)
);

CREATE INDEX IF NOT EXISTS idx_packages_name ON packages(ecosystem, name);

CREATE TABLE IF NOT EXISTS package_versions (
    id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL REFERENCES packages(id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    manifest_id TEXT NOT NULL,
    cksum TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    metadata_json TEXT NOT NULL,
    is_yanked BOOLEAN NOT NULL DEFAULT FALSE,
    created_by_user_id TEXT NOT NULL REFERENCES users(id),
    created_at_nanos BIGINT NOT NULL,
    UNIQUE(package_id, version)
);

CREATE INDEX IF NOT EXISTS idx_package_versions_pkg ON package_versions(package_id);
