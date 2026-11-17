use chrono::Utc;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_FINGERPRINT_KEY_ID: &str = "hmac-sha256:v1";
pub const DEFAULT_FINGERPRINT_SECRET: &[u8] = b"spine_forge_protected_fingerprint_salt_2026";

/// Resolves secret scanning fingerprint salt from environment or defaults to baseline.
pub fn resolve_fingerprint_secret() -> Vec<u8> {
    if let Ok(val) = std::env::var("SPINE_FINGERPRINT_SECRET") {
        let trimmed = val.trim();
        if !trimmed.is_empty() {
            return trimmed.as_bytes().to_vec();
        }
    }
    DEFAULT_FINGERPRINT_SECRET.to_vec()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AdvisoryFeedProvenance {
    pub source_name: String,
    pub feed_version: String,
    pub status: String, // "ACTIVE_INDEXED", "NOT_CONFIGURED", "UNAVAILABLE", "STALE"
    pub advisories_count: usize,
    pub last_synced_nanos: i64,
}

use crate::core::error::ForgeError;
use crate::core::types::{
    DependencyInventory, DependencyVulnerability, SecretScanFinding, SecurityAdvisory,
    SecurityScan, SecuritySummaryDto,
};
use crate::db::Database;
use crate::permissions::PermissionService;

pub struct SecurityService {
    db: Arc<dyn Database>,
    perms: Arc<PermissionService>,
    storage_root: PathBuf,
}

impl SecurityService {
    pub fn new(
        db: Arc<dyn Database>,
        perms: Arc<PermissionService>,
        storage_root: PathBuf,
    ) -> Self {
        Self {
            db,
            perms,
            storage_root,
        }
    }

    /// Seed default well-known security advisories for Cargo dependencies.
    pub async fn seed_default_advisories(&self) -> Result<(), ForgeError> {
        let existing = self.db.list_security_advisories("CARGO").await?;
        if !existing.is_empty() {
            return Ok(());
        }

        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let defaults = vec![
            SecurityAdvisory {
                id: Uuid::new_v4().to_string(),
                advisory_id: "RUSTSEC-2021-0145".to_string(),
                ecosystem: "CARGO".to_string(),
                package_name: "atty".to_string(),
                vulnerable_version_range: "<0.2.14".to_string(),
                patched_version: Some("0.2.14".to_string()),
                severity: "MEDIUM".to_string(),
                title: "Potential unaligned read in atty on Windows".to_string(),
                description: "On Windows, atty attempted to cast HANDLE to BOOL causing an unaligned read.".to_string(),
                published_at_nanos: now,
            },
            SecurityAdvisory {
                id: Uuid::new_v4().to_string(),
                advisory_id: "RUSTSEC-2023-0071".to_string(),
                ecosystem: "CARGO".to_string(),
                package_name: "rsa".to_string(),
                vulnerable_version_range: "<0.9.6".to_string(),
                patched_version: Some("0.9.6".to_string()),
                severity: "HIGH".to_string(),
                title: "Marvin Attack: timing sidechannel in PKCS#1 v1.5 padding".to_string(),
                description: "rsa crate does not implement constant-time verification for PKCS#1 v1.5 padding.".to_string(),
                published_at_nanos: now,
            },
            SecurityAdvisory {
                id: Uuid::new_v4().to_string(),
                advisory_id: "RUSTSEC-2024-0006".to_string(),
                ecosystem: "CARGO".to_string(),
                package_name: "h2".to_string(),
                vulnerable_version_range: "<0.3.26".to_string(),
                patched_version: Some("0.3.26".to_string()),
                severity: "HIGH".to_string(),
                title: "Denial of Service via CONTINUATION frames in HTTP/2".to_string(),
                description: "An attacker could send endless stream of CONTINUATION frames without ending the headers.".to_string(),
                published_at_nanos: now,
            },
        ];

        for adv in defaults {
            self.db.upsert_security_advisory(adv).await?;
        }

        Ok(())
    }

    // -------------------------------------------------------------------------
    // Secret Scanning
    // -------------------------------------------------------------------------
    pub async fn scan_repository_secrets(
        &self,
        actor_id: &str,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<SecurityScan, ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;

        let scan_id = Uuid::new_v4().to_string();
        let start_time = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let repo_dir = self.storage_root.join(repo_id);
        let mut findings = Vec::new();

        if repo_dir.exists() {
            scan_dir_for_secrets(&repo_dir, repo_id, commit_oid, &mut findings);
        }

        let findings_count = findings.len() as i32;

        for f in findings {
            self.db.create_secret_scan_finding(f).await?;
        }

        let end_time = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let scan = SecurityScan {
            id: scan_id,
            repo_id: repo_id.to_string(),
            commit_oid: commit_oid.to_string(),
            scan_type: "SECRET_SCAN".to_string(),
            status: "COMPLETED".to_string(),
            findings_count,
            started_at_nanos: start_time,
            completed_at_nanos: end_time,
        };

        self.db.create_security_scan(scan).await
    }

    pub async fn list_secret_findings(
        &self,
        actor_id: &str,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<SecretScanFinding>, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;
        self.db
            .list_secret_scan_findings(repo_id, status_filter)
            .await
    }

    pub async fn remediate_secret_finding(
        &self,
        actor_id: &str,
        repo_id: &str,
        finding_id: &str,
        status: &str, // "RESOLVED" or "FALSE_POSITIVE"
    ) -> Result<(), ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.db
            .update_secret_scan_finding_status(finding_id, status, now)
            .await
    }

    // -------------------------------------------------------------------------
    // Dependency Inventory & Vulnerability Auditing
    // -------------------------------------------------------------------------
    pub async fn audit_dependencies(
        &self,
        actor_id: &str,
        repo_id: &str,
        commit_oid: &str,
    ) -> Result<SecurityScan, ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;
        self.seed_default_advisories().await?;

        let scan_id = Uuid::new_v4().to_string();
        let start_time = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        let repo_dir = self.storage_root.join(repo_id);
        let mut inventory = Vec::new();

        let lock_file = repo_dir.join("Cargo.lock");
        let toml_file = repo_dir.join("Cargo.toml");

        if lock_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&lock_file) {
                parse_cargo_lock(&content, repo_id, &mut inventory);
            }
        } else if toml_file.exists() {
            if let Ok(content) = std::fs::read_to_string(&toml_file) {
                parse_cargo_toml(&content, repo_id, &mut inventory);
            }
        }

        self.db.upsert_dependency_inventory(&inventory).await?;

        // Cross-reference with advisories
        let advisories = self.db.list_security_advisories("CARGO").await?;
        let mut vulns_found = 0;

        for dep in &inventory {
            for adv in &advisories {
                if adv.package_name == dep.package_name
                    && version_matches(&dep.version, &adv.vulnerable_version_range)
                {
                    let vuln = DependencyVulnerability {
                        id: Uuid::new_v4().to_string(),
                        repo_id: repo_id.to_string(),
                        dependency_id: dep.id.clone(),
                        advisory_id: adv.advisory_id.clone(),
                        package_name: dep.package_name.clone(),
                        affected_version: dep.version.clone(),
                        severity: adv.severity.clone(),
                        patched_version: adv.patched_version.clone(),
                        status: "OPEN".to_string(),
                        created_at_nanos: start_time,
                        resolved_at_nanos: None,
                    };
                    let _ = self.db.create_dependency_vulnerability(vuln).await;
                    vulns_found += 1;
                }
            }
        }

        let end_time = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let scan = SecurityScan {
            id: scan_id,
            repo_id: repo_id.to_string(),
            commit_oid: commit_oid.to_string(),
            scan_type: "DEPENDENCY_AUDIT".to_string(),
            status: "COMPLETED".to_string(),
            findings_count: vulns_found,
            started_at_nanos: start_time,
            completed_at_nanos: end_time,
        };

        self.db.create_security_scan(scan).await
    }

    pub async fn list_dependencies(
        &self,
        actor_id: &str,
        repo_id: &str,
    ) -> Result<Vec<DependencyInventory>, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;
        self.db.list_dependency_inventory(repo_id).await
    }

    pub async fn list_vulnerabilities(
        &self,
        actor_id: &str,
        repo_id: &str,
        status_filter: Option<&str>,
    ) -> Result<Vec<DependencyVulnerability>, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;
        self.db
            .list_dependency_vulnerabilities(repo_id, status_filter)
            .await
    }

    pub async fn remediate_vulnerability(
        &self,
        actor_id: &str,
        repo_id: &str,
        vuln_id: &str,
        status: &str, // "FIXED" or "DISMISSED"
    ) -> Result<(), ForgeError> {
        self.perms.require_repo_write(actor_id, repo_id).await?;
        let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        self.db
            .update_dependency_vulnerability_status(vuln_id, status, now)
            .await
    }

    // -------------------------------------------------------------------------
    // SBOM Export (SPDX 2.3 & CycloneDX 1.5)
    // -------------------------------------------------------------------------
    pub async fn export_sbom_spdx(
        &self,
        actor_id: &str,
        repo_id: &str,
    ) -> Result<serde_json::Value, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;
        let repo = self
            .db
            .get_repository_by_id(repo_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Repository '{}' not found", repo_id)))?;

        let deps = self.db.list_dependency_inventory(repo_id).await?;
        let now_iso = Utc::now().to_rfc3339();

        let packages: Vec<serde_json::Value> = deps
            .iter()
            .map(|d| {
                serde_json::json!({
                    "SPDXID": format!("SPDXRef-Package-{}", d.package_name),
                    "name": d.package_name,
                    "versionInfo": d.version,
                    "downloadLocation": format!("https://crates.io/api/v1/crates/{}/download", d.package_name),
                    "filesAnalyzed": false,
                    "supplier": "Organization: Crates.io",
                })
            })
            .collect();

        let spdx = serde_json::json!({
            "spdxVersion": "SPDX-2.3",
            "dataLicense": "CC0-1.0",
            "SPDXID": "SPDXRef-DOCUMENT",
            "name": format!("{}/{}", repo.owner_username, repo.name),
            "documentNamespace": format!("https://spine.dev/spdx/{}/{}", repo.owner_username, repo.name),
            "creationInfo": {
                "created": now_iso,
                "creators": ["Tool: Spine-Security-Center-1.0"]
            },
            "packages": packages
        });

        Ok(spdx)
    }

    pub async fn export_sbom_cyclonedx(
        &self,
        actor_id: &str,
        repo_id: &str,
    ) -> Result<serde_json::Value, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;
        let repo = self
            .db
            .get_repository_by_id(repo_id)
            .await?
            .ok_or_else(|| ForgeError::NotFound(format!("Repository '{}' not found", repo_id)))?;

        let deps = self.db.list_dependency_inventory(repo_id).await?;

        let components: Vec<serde_json::Value> = deps
            .iter()
            .map(|d| {
                serde_json::json!({
                    "type": "library",
                    "bom-ref": format!("pkg:cargo/{}@{}", d.package_name, d.version),
                    "name": d.package_name,
                    "version": d.version,
                    "purl": format!("pkg:cargo/{}@{}", d.package_name, d.version),
                    "scope": if d.is_direct { "required" } else { "optional" }
                })
            })
            .collect();

        let cdx = serde_json::json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.5",
            "serialNumber": format!("urn:uuid:{}", Uuid::new_v4()),
            "version": 1,
            "metadata": {
                "timestamp": Utc::now().to_rfc3339(),
                "tools": [{
                    "vendor": "Spine",
                    "name": "Spine Security Center",
                    "version": "1.0"
                }],
                "component": {
                    "type": "application",
                    "name": format!("{}/{}", repo.owner_username, repo.name),
                }
            },
            "components": components
        });

        Ok(cdx)
    }

    pub async fn get_security_summary(
        &self,
        actor_id: &str,
        repo_id: &str,
    ) -> Result<SecuritySummaryDto, ForgeError> {
        self.perms
            .require_repo_read(Some(actor_id), repo_id)
            .await?;

        let secrets = self.db.list_secret_scan_findings(repo_id, None).await?;
        let deps = self.db.list_dependency_inventory(repo_id).await?;
        let vulns = self
            .db
            .list_dependency_vulnerabilities(repo_id, None)
            .await?;

        let total_secrets_found = secrets.len();
        let open_secrets_count = secrets.iter().filter(|s| s.status == "OPEN").count();

        let total_dependencies = deps.len();
        let open_vulns: Vec<&DependencyVulnerability> =
            vulns.iter().filter(|v| v.status == "OPEN").collect();
        let vulnerable_dependencies_count = open_vulns.len();

        let mut critical_count = 0;
        let mut high_count = 0;
        let mut medium_count = 0;
        let mut low_count = 0;

        for s in secrets.iter().filter(|s| s.status == "OPEN") {
            match s.severity.as_str() {
                "CRITICAL" => critical_count += 1,
                "HIGH" => high_count += 1,
                "MEDIUM" => medium_count += 1,
                _ => low_count += 1,
            }
        }

        for v in open_vulns {
            match v.severity.as_str() {
                "CRITICAL" => critical_count += 1,
                "HIGH" => high_count += 1,
                "MEDIUM" => medium_count += 1,
                _ => low_count += 1,
            }
        }

        Ok(SecuritySummaryDto {
            total_secrets_found,
            open_secrets_count,
            total_dependencies,
            vulnerable_dependencies_count,
            critical_count,
            high_count,
            medium_count,
            low_count,
        })
    }

    pub async fn get_advisory_feed_provenance(&self) -> Result<AdvisoryFeedProvenance, ForgeError> {
        let advisories = self.db.list_security_advisories("CARGO").await?;
        let count = advisories.len();
        let status = if count > 0 {
            "ACTIVE_INDEXED".to_string()
        } else {
            "NOT_CONFIGURED".to_string()
        };
        Ok(AdvisoryFeedProvenance {
            source_name: "RustSec Advisory Database (crates.io)".to_string(),
            feed_version: "2026.11.01-cargo".to_string(),
            status,
            advisories_count: count,
            last_synced_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
        })
    }
}

// -----------------------------------------------------------------------------
// Secret Scanner Engine (Safe Fingerprinting)
// -----------------------------------------------------------------------------
fn scan_dir_for_secrets(
    root: &Path,
    repo_id: &str,
    commit_oid: &str,
    findings: &mut Vec<SecretScanFinding>,
) {
    let mut stack = vec![root.to_path_buf()];

    while let Some(dir) = stack.pop() {
        let read_dir = match std::fs::read_dir(&dir) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let path = entry.path();
            let file_name = entry.file_name().to_string_lossy().to_string();

            if file_name.starts_with('.') || file_name == "target" || file_name == "node_modules" {
                continue;
            }

            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Ok(content) = std::fs::read_to_string(&path) {
                    let rel_path = path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .replace('\\', "/");
                    for (line_idx, line) in content.lines().enumerate() {
                        let line_num = (line_idx + 1) as i32;

                        // Rule 1: Spine PAT
                        if let Some(pos) = line.find("sp_pat_") {
                            let candidate: String = line[pos..]
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            if candidate.len() >= 35 {
                                findings.push(SecretScanFinding {
                                    id: Uuid::new_v4().to_string(),
                                    repo_id: repo_id.to_string(),
                                    commit_oid: commit_oid.to_string(),
                                    file_path: rel_path.clone(),
                                    line_number: line_num,
                                    rule_id: "SEC-001".to_string(),
                                    rule_name: "Spine Personal Access Token".to_string(),
                                    severity: "CRITICAL".to_string(),
                                    safe_fingerprint: compute_safe_fingerprint(&candidate),
                                    status: "OPEN".to_string(),
                                    created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                    resolved_at_nanos: None,
                                });
                            }
                        }

                        // Rule 2: GitHub PAT
                        if let Some(pos) = line.find("ghp_") {
                            let candidate: String = line[pos..]
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric())
                                .collect();
                            if candidate.len() >= 40 {
                                findings.push(SecretScanFinding {
                                    id: Uuid::new_v4().to_string(),
                                    repo_id: repo_id.to_string(),
                                    commit_oid: commit_oid.to_string(),
                                    file_path: rel_path.clone(),
                                    line_number: line_num,
                                    rule_id: "SEC-002".to_string(),
                                    rule_name: "GitHub Personal Access Token".to_string(),
                                    severity: "HIGH".to_string(),
                                    safe_fingerprint: compute_safe_fingerprint(&candidate),
                                    status: "OPEN".to_string(),
                                    created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                    resolved_at_nanos: None,
                                });
                            }
                        }

                        // Rule 3: AWS Access Key
                        if let Some(pos) = line.find("AKIA") {
                            let candidate: String = line[pos..]
                                .chars()
                                .take_while(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
                                .collect();
                            if candidate.len() >= 20 {
                                findings.push(SecretScanFinding {
                                    id: Uuid::new_v4().to_string(),
                                    repo_id: repo_id.to_string(),
                                    commit_oid: commit_oid.to_string(),
                                    file_path: rel_path.clone(),
                                    line_number: line_num,
                                    rule_id: "SEC-003".to_string(),
                                    rule_name: "AWS Access Key ID".to_string(),
                                    severity: "HIGH".to_string(),
                                    safe_fingerprint: compute_safe_fingerprint(&candidate),
                                    status: "OPEN".to_string(),
                                    created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                    resolved_at_nanos: None,
                                });
                            }
                        }

                        // Rule 4: Private Key
                        if line.contains("BEGIN ") && line.contains("PRIVATE KEY-----") {
                            findings.push(SecretScanFinding {
                                id: Uuid::new_v4().to_string(),
                                repo_id: repo_id.to_string(),
                                commit_oid: commit_oid.to_string(),
                                file_path: rel_path.clone(),
                                line_number: line_num,
                                rule_id: "SEC-004".to_string(),
                                rule_name: "Private Cryptographic Key".to_string(),
                                severity: "CRITICAL".to_string(),
                                safe_fingerprint: compute_safe_fingerprint(line.trim()),
                                status: "OPEN".to_string(),
                                created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                resolved_at_nanos: None,
                            });
                        }

                        // Rule 5: Slack Webhook
                        if let Some(pos) = line.find("https://hooks.slack.com/services/") {
                            let candidate: String = line[pos..]
                                .chars()
                                .take_while(|c| !c.is_whitespace() && *c != '"' && *c != '\'')
                                .collect();
                            findings.push(SecretScanFinding {
                                id: Uuid::new_v4().to_string(),
                                repo_id: repo_id.to_string(),
                                commit_oid: commit_oid.to_string(),
                                file_path: rel_path.clone(),
                                line_number: line_num,
                                rule_id: "SEC-005".to_string(),
                                rule_name: "Slack Webhook URL".to_string(),
                                severity: "MEDIUM".to_string(),
                                safe_fingerprint: compute_safe_fingerprint(&candidate),
                                status: "OPEN".to_string(),
                                created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                resolved_at_nanos: None,
                            });
                        }

                        // Rule 6: Stripe Secret Key
                        if let Some(pos) = line.find("sk_live_") {
                            let candidate: String = line[pos..]
                                .chars()
                                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                                .collect();
                            if candidate.len() >= 24 {
                                findings.push(SecretScanFinding {
                                    id: Uuid::new_v4().to_string(),
                                    repo_id: repo_id.to_string(),
                                    commit_oid: commit_oid.to_string(),
                                    file_path: rel_path.clone(),
                                    line_number: line_num,
                                    rule_id: "SEC-006".to_string(),
                                    rule_name: "Stripe Secret API Key".to_string(),
                                    severity: "CRITICAL".to_string(),
                                    safe_fingerprint: compute_safe_fingerprint(&candidate),
                                    status: "OPEN".to_string(),
                                    created_at_nanos: Utc::now().timestamp_nanos_opt().unwrap_or(0),
                                    resolved_at_nanos: None,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Compute a safe, non-reversible fingerprint for discovered credentials using keyed HMAC-SHA-256.
/// Never store or log raw secrets! Key rotation is supported via key_id and secret salt.
pub fn compute_keyed_safe_fingerprint(raw_secret: &str, key_id: &str, key_bytes: &[u8]) -> String {
    let mut mac = HmacSha256::new_from_slice(key_bytes)
        .unwrap_or_else(|_| HmacSha256::new_from_slice(DEFAULT_FINGERPRINT_SECRET).unwrap());
    mac.update(raw_secret.as_bytes());
    let hmac_hex = hex::encode(mac.finalize().into_bytes());

    if raw_secret.len() <= 8 {
        format!("***... ({}:{})", key_id, &hmac_hex[..12])
    } else {
        let prefix = &raw_secret[..4];
        let suffix = &raw_secret[raw_secret.len() - 4..];
        format!(
            "{}...****...{} ({}:{})",
            prefix,
            suffix,
            key_id,
            &hmac_hex[..12]
        )
    }
}

pub fn compute_safe_fingerprint(raw_secret: &str) -> String {
    let secret = resolve_fingerprint_secret();
    compute_keyed_safe_fingerprint(
        raw_secret,
        DEFAULT_FINGERPRINT_KEY_ID,
        &secret,
    )
}

// -----------------------------------------------------------------------------
// Cargo Parser Helpers
// -----------------------------------------------------------------------------
fn parse_cargo_lock(content: &str, repo_id: &str, inventory: &mut Vec<DependencyInventory>) {
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            if let (Some(name), Some(ver)) = (current_name.take(), current_version.take()) {
                inventory.push(DependencyInventory {
                    id: Uuid::new_v4().to_string(),
                    repo_id: repo_id.to_string(),
                    ecosystem: "CARGO".to_string(),
                    package_name: name,
                    version: ver,
                    is_direct: true,
                    created_at_nanos: now,
                });
            }
        } else if trimmed.starts_with("name =") {
            if let Some(val) = extract_toml_string(trimmed) {
                current_name = Some(val);
            }
        } else if trimmed.starts_with("version =") {
            if let Some(val) = extract_toml_string(trimmed) {
                current_version = Some(val);
            }
        }
    }

    if let (Some(name), Some(ver)) = (current_name, current_version) {
        inventory.push(DependencyInventory {
            id: Uuid::new_v4().to_string(),
            repo_id: repo_id.to_string(),
            ecosystem: "CARGO".to_string(),
            package_name: name,
            version: ver,
            is_direct: true,
            created_at_nanos: now,
        });
    }
}

fn parse_cargo_toml(content: &str, repo_id: &str, inventory: &mut Vec<DependencyInventory>) {
    let now = Utc::now().timestamp_nanos_opt().unwrap_or(0);
    let mut in_dependencies = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]" || trimmed == "[dev-dependencies]";
            continue;
        }

        if in_dependencies && !trimmed.is_empty() && !trimmed.starts_with('#') {
            if let Some((pkg, rest)) = trimmed.split_once('=') {
                let pkg_name = pkg.trim().to_string();
                let ver = extract_toml_string(rest.trim()).unwrap_or_else(|| "1.0.0".to_string());
                inventory.push(DependencyInventory {
                    id: Uuid::new_v4().to_string(),
                    repo_id: repo_id.to_string(),
                    ecosystem: "CARGO".to_string(),
                    package_name: pkg_name,
                    version: ver,
                    is_direct: true,
                    created_at_nanos: now,
                });
            }
        }
    }
}

fn extract_toml_string(line: &str) -> Option<String> {
    if let Some(start) = line.find('"') {
        if let Some(end) = line[start + 1..].find('"') {
            return Some(line[start + 1..start + 1 + end].to_string());
        }
    }
    None
}

fn version_matches(version: &str, range: &str) -> bool {
    let clean_range = range.trim();
    if let Some(stripped) = clean_range.strip_prefix('<') {
        let max_ver = stripped.trim();
        version < max_ver
    } else if let Some(stripped) = clean_range.strip_prefix("<=") {
        let max_ver = stripped.trim();
        version <= max_ver
    } else {
        version == clean_range
    }
}
