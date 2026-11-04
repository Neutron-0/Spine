-- 0007_releases_and_packages.sql
-- Releases, release assets, packages, and package versions (e.g. Cargo registry).

CREATE TABLE IF NOT EXISTS releases (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    tag_name TEXT NOT NULL,
    target_commit_id TEXT NOT NULL,
    title TEXT NOT NULL,
    body TEXT,
    is_draft BOOLEAN NOT NULL DEFAULT FALSE,
    is_prerelease BOOLEAN NOT NULL DEFAULT FALSE,
    author_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at_nanos BIGINT NOT NULL,
    published_at_nanos BIGINT,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_repo_release_tag UNIQUE (repo_id, tag_name)
);

CREATE INDEX IF NOT EXISTS idx_releases_repo ON releases (repo_id);

CREATE TABLE IF NOT EXISTS release_assets (
    id TEXT PRIMARY KEY,
    release_id TEXT NOT NULL REFERENCES releases (id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    manifest_id TEXT NOT NULL REFERENCES artifact_manifests (id) ON DELETE RESTRICT,
    whole_hash TEXT NOT NULL,
    downloads_count BIGINT NOT NULL DEFAULT 0,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_release_asset_name UNIQUE (release_id, name)
);

CREATE INDEX IF NOT EXISTS idx_release_assets_rel ON release_assets (release_id);

CREATE TABLE IF NOT EXISTS packages (
    id TEXT PRIMARY KEY,
    ecosystem TEXT NOT NULL,
    name TEXT NOT NULL,
    owner_type TEXT NOT NULL,
    owner_id TEXT NOT NULL,
    is_private BOOLEAN NOT NULL DEFAULT FALSE,
    description TEXT,
    created_at_nanos BIGINT NOT NULL,
    updated_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_pkg_eco_name UNIQUE (ecosystem, name)
);

CREATE INDEX IF NOT EXISTS idx_pkgs_owner ON packages (owner_id);

CREATE TABLE IF NOT EXISTS package_versions (
    id TEXT PRIMARY KEY,
    package_id TEXT NOT NULL REFERENCES packages (id) ON DELETE CASCADE,
    version TEXT NOT NULL,
    manifest_id TEXT NOT NULL REFERENCES artifact_manifests (id) ON DELETE RESTRICT,
    cksum TEXT NOT NULL,
    size_bytes BIGINT NOT NULL,
    metadata_json TEXT NOT NULL,
    is_yanked BOOLEAN NOT NULL DEFAULT FALSE,
    created_by_user_id TEXT NOT NULL REFERENCES users (id) ON DELETE RESTRICT,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_pkg_ver UNIQUE (package_id, version)
);

CREATE INDEX IF NOT EXISTS idx_pkg_versions_pkg ON package_versions (package_id);
