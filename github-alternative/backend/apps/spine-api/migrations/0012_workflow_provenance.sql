-- 0012_workflow_provenance.sql
-- Persist authenticated workflow provenance on workflow_runs and ensure cluster maintenance barrier table exists.

ALTER TABLE workflow_runs ADD COLUMN IF NOT EXISTS source_repo_id TEXT;
ALTER TABLE workflow_runs ADD COLUMN IF NOT EXISTS is_fork BOOLEAN NOT NULL DEFAULT FALSE;
ALTER TABLE workflow_runs ADD COLUMN IF NOT EXISTS trust_classification TEXT NOT NULL DEFAULT 'Untrusted';
ALTER TABLE workflow_runs ADD COLUMN IF NOT EXISTS provenance_reason TEXT NOT NULL DEFAULT '';

CREATE TABLE IF NOT EXISTS _cluster_maintenance_barrier (
    id INT PRIMARY KEY DEFAULT 1,
    is_active BOOLEAN NOT NULL DEFAULT FALSE,
    activated_at_nanos BIGINT NOT NULL DEFAULT 0,
    activated_by VARCHAR(255) NOT NULL DEFAULT '',
    reason VARCHAR(255) NOT NULL DEFAULT ''
);

INSERT INTO _cluster_maintenance_barrier (id, is_active, activated_at_nanos, activated_by, reason)
VALUES (1, FALSE, 0, 'init', 'System initialized')
ON CONFLICT (id) DO NOTHING;
