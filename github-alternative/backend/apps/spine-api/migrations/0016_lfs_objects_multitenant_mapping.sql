-- 0016_lfs_objects_multitenant_mapping.sql
-- Create repository-scoped LFS objects mapping table, unique constraints, indexes,
-- and safely backfill legacy unmapped LFS manifests into lfs_objects with strict ambiguity validation.

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

DO $$
DECLARE
    m RECORD;
    target_repo_id TEXT;
    new_id TEXT;
BEGIN
    FOR m IN
        SELECT am.id, am.artifact_id, am.total_size, am.manifest_json, am.created_at_nanos
        FROM artifact_manifests am
        WHERE NOT EXISTS (
            SELECT 1 FROM lfs_objects lo WHERE lo.manifest_id = am.id
        )
        AND NOT EXISTS (
            SELECT 1 FROM artifact_versions av WHERE av.manifest_id = am.id
        )
        AND NOT EXISTS (
            SELECT 1 FROM release_assets ra WHERE ra.manifest_id = am.id
        )
        AND NOT EXISTS (
            SELECT 1 FROM package_versions pv WHERE pv.manifest_id = am.id
        )
        AND (
            (length(am.artifact_id) = 64 AND am.artifact_id ~ '^[0-9a-fA-F]{64}$')
            OR (am.manifest_json::jsonb ? 'repository_id')
            OR (am.manifest_json::jsonb ? 'repo_id')
        )
    LOOP
        target_repo_id := NULL;
        IF m.manifest_json::jsonb ? 'repository_id' THEN
            target_repo_id := m.manifest_json::jsonb->>'repository_id';
        ELSIF m.manifest_json::jsonb ? 'repo_id' THEN
            target_repo_id := m.manifest_json::jsonb->>'repo_id';
        END IF;

        IF length(m.artifact_id) != 64 OR m.artifact_id !~ '^[0-9a-fA-F]{64}$' THEN
            RAISE EXCEPTION 'Invalid legacy LFS manifest %: artifact_id "%" is not a valid 64-hex SHA-256 OID', m.id, m.artifact_id;
        END IF;

        IF target_repo_id IS NULL THEN
            RAISE EXCEPTION 'Ambiguous legacy LFS manifest %: manifest for OID "%" lacks explicit repository_id metadata. Refusing to infer repository ownership.', m.id, m.artifact_id;
        END IF;

        IF NOT EXISTS (SELECT 1 FROM forge_repositories WHERE id = target_repo_id) THEN
            RAISE EXCEPTION 'Invalid legacy LFS manifest %: target repository "%" does not exist in forge_repositories', m.id, target_repo_id;
        END IF;

        IF EXISTS (SELECT 1 FROM lfs_objects WHERE repository_id = target_repo_id AND oid = lower(m.artifact_id) AND manifest_id != m.id) THEN
            RAISE EXCEPTION 'Duplicate ambiguous legacy LFS mapping for repository "%" and OID "%": multiple distinct manifests exist', target_repo_id, lower(m.artifact_id);
        END IF;

        new_id := 'lfs_' || substr(md5(random()::text || clock_timestamp()::text), 1, 24);
        INSERT INTO lfs_objects (id, repository_id, oid, size_bytes, manifest_id, created_at_nanos)
        VALUES (new_id, target_repo_id, lower(m.artifact_id), m.total_size, m.id, m.created_at_nanos)
        ON CONFLICT (repository_id, oid) DO NOTHING;
    END LOOP;
END $$;
