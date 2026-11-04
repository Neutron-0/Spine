-- 0011_security_center.sql
-- Security scans, secret scan findings, security advisories, dependency inventories, and vulnerabilities.

CREATE TABLE IF NOT EXISTS security_scans (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    commit_oid TEXT NOT NULL,
    scan_type TEXT NOT NULL,
    status TEXT NOT NULL,
    findings_count INT NOT NULL DEFAULT 0,
    started_at_nanos BIGINT NOT NULL,
    completed_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_security_scans_repo ON security_scans (repo_id, started_at_nanos DESC);

CREATE TABLE IF NOT EXISTS secret_scan_findings (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    commit_oid TEXT NOT NULL,
    file_path TEXT NOT NULL,
    line_number INT NOT NULL,
    rule_id TEXT NOT NULL,
    rule_name TEXT NOT NULL,
    severity TEXT NOT NULL,
    safe_fingerprint TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'OPEN',
    created_at_nanos BIGINT NOT NULL,
    resolved_at_nanos BIGINT
);

CREATE INDEX IF NOT EXISTS idx_secret_findings_repo ON secret_scan_findings (repo_id, status);

CREATE TABLE IF NOT EXISTS security_advisories (
    id TEXT PRIMARY KEY,
    advisory_id TEXT NOT NULL UNIQUE,
    ecosystem TEXT NOT NULL,
    package_name TEXT NOT NULL,
    vulnerable_version_range TEXT NOT NULL,
    patched_version TEXT,
    severity TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    published_at_nanos BIGINT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_advisories_pkg ON security_advisories (ecosystem, package_name);

CREATE TABLE IF NOT EXISTS dependency_inventories (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    ecosystem TEXT NOT NULL,
    package_name TEXT NOT NULL,
    version TEXT NOT NULL,
    is_direct BOOLEAN NOT NULL DEFAULT TRUE,
    created_at_nanos BIGINT NOT NULL,
    CONSTRAINT uq_dep_repo_pkg_ver UNIQUE (repo_id, ecosystem, package_name, version)
);

CREATE INDEX IF NOT EXISTS idx_dep_inv_repo ON dependency_inventories (repo_id);

CREATE TABLE IF NOT EXISTS dependency_vulnerabilities (
    id TEXT PRIMARY KEY,
    repo_id TEXT NOT NULL REFERENCES forge_repositories (id) ON DELETE CASCADE,
    dependency_id TEXT NOT NULL REFERENCES dependency_inventories (id) ON DELETE CASCADE,
    advisory_id TEXT NOT NULL,
    package_name TEXT NOT NULL,
    affected_version TEXT NOT NULL,
    severity TEXT NOT NULL,
    patched_version TEXT,
    status TEXT NOT NULL DEFAULT 'OPEN',
    created_at_nanos BIGINT NOT NULL,
    resolved_at_nanos BIGINT,
    CONSTRAINT uq_dep_vuln UNIQUE (repo_id, dependency_id, advisory_id)
);

CREATE INDEX IF NOT EXISTS idx_dep_vulns_repo ON dependency_vulnerabilities (repo_id, status);
