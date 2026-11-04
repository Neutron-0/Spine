use std::fs;
use std::sync::Arc;
use tempfile::tempdir;

use spine_api::{
    core::types::*,
    db::{Database, InMemoryDatabase},
    AppState,
};

#[tokio::test]
async fn test_security_center_secret_scanning_and_sbom_audit() {
    let tmp = tempdir().unwrap();
    let storage_root = tmp.path().join("storage");
    fs::create_dir_all(&storage_root).unwrap();

    let db = Arc::new(InMemoryDatabase::new());
    let state = AppState::new_with_defaults(db.clone(), &storage_root, None).unwrap();

    let dev = User {
        id: "user-dev".to_string(),
        username: "devsec".to_string(),
        normalized_username: "devsec".to_string(),
        email: "dev@example.com".to_string(),
        normalized_email: "dev@example.com".to_string(),
        display_name: "Security Engineer".to_string(),
        password_hash: "hash".to_string(),
        account_status: "ACTIVE".to_string(),
        avatar_url: None,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        last_login_nanos: None,
    };
    db.create_user(dev).await.unwrap();

    let repo = ForgeRepository {
        id: "repo-sec".to_string(),
        owner_user_id: "user-dev".to_string(),
        owner_username: "devsec".to_string(),
        owner_org_id: None,
        name: "target-service".to_string(),
        normalized_name: "target-service".to_string(),
        description: "Target repository for security audit".to_string(),
        is_private: false,
        default_branch: "main".to_string(),
        native_repo_id: "repo-sec".to_string(),
        durability_policy: spine_cluster::durability::DurabilityPolicy::SingleCopy,
        provisioning_status: ProvisioningStatus::Ready,
        created_at_nanos: 1,
        updated_at_nanos: 1,
        archived: false,
    };
    db.create_repository(repo).await.unwrap();

    let repo_dir = storage_root.join("repo-sec");
    fs::create_dir_all(&repo_dir).unwrap();

    // 1. Plant a dummy credential and a vulnerable Cargo.lock
    let fake_token = "sp_pat_999988887777666655554444333322221111";
    fs::write(
        repo_dir.join("config.env"),
        format!(
            "DATABASE_URL=postgres://localhost\nSPINE_TOKEN={}\n",
            fake_token
        ),
    )
    .unwrap();

    let cargo_lock_content = r#"
version = 3

[[package]]
name = "atty"
version = "0.2.13"

[[package]]
name = "serde"
version = "1.0.197"
"#;
    fs::write(repo_dir.join("Cargo.lock"), cargo_lock_content).unwrap();

    // 2. Execute Secret Scan
    let scan_res = state
        .security
        .scan_repository_secrets("user-dev", "repo-sec", "head-commit")
        .await
        .unwrap();

    assert_eq!(scan_res.scan_type, "SECRET_SCAN");
    assert_eq!(scan_res.findings_count, 1);

    let findings = state
        .security
        .list_secret_findings("user-dev", "repo-sec", None)
        .await
        .unwrap();

    assert_eq!(findings.len(), 1);
    let finding = &findings[0];
    assert_eq!(finding.rule_id, "SEC-001");
    assert_eq!(finding.severity, "CRITICAL");
    assert_eq!(finding.status, "OPEN");

    // CRITICAL: Safe Fingerprinting Guarantee - NEVER raw secret!
    assert!(
        !finding.safe_fingerprint.contains(fake_token),
        "Safe fingerprint must never contain the plaintext token!"
    );
    assert!(finding.safe_fingerprint.starts_with("sp_p"));
    assert!(finding.safe_fingerprint.contains("...****..."));
    assert!(finding.safe_fingerprint.contains("sha256:"));

    // 3. Remediate Secret Finding
    state
        .security
        .remediate_secret_finding("user-dev", "repo-sec", &finding.id, "RESOLVED")
        .await
        .unwrap();

    let open_findings = state
        .security
        .list_secret_findings("user-dev", "repo-sec", Some("OPEN"))
        .await
        .unwrap();
    assert_eq!(open_findings.len(), 0);

    // 4. Execute Dependency Audit
    let dep_scan = state
        .security
        .audit_dependencies("user-dev", "repo-sec", "head-commit")
        .await
        .unwrap();

    assert_eq!(dep_scan.scan_type, "DEPENDENCY_AUDIT");
    assert!(
        dep_scan.findings_count >= 1,
        "Should find atty < 0.2.14 vulnerability"
    );

    let vulns = state
        .security
        .list_vulnerabilities("user-dev", "repo-sec", Some("OPEN"))
        .await
        .unwrap();

    assert_eq!(vulns.len(), 1);
    assert_eq!(vulns[0].package_name, "atty");
    assert_eq!(vulns[0].affected_version, "0.2.13");
    assert_eq!(vulns[0].advisory_id, "RUSTSEC-2021-0145");
    assert_eq!(vulns[0].patched_version.as_deref(), Some("0.2.14"));

    // 5. Test SPDX 2.3 SBOM Export
    let spdx = state
        .security
        .export_sbom_spdx("user-dev", "repo-sec")
        .await
        .unwrap();

    assert_eq!(spdx["spdxVersion"], "SPDX-2.3");
    let packages = spdx["packages"].as_array().expect("packages array");
    assert_eq!(packages.len(), 2);
    assert!(packages
        .iter()
        .any(|p| p["name"] == "atty" && p["versionInfo"] == "0.2.13"));
    assert!(packages
        .iter()
        .any(|p| p["name"] == "serde" && p["versionInfo"] == "1.0.197"));

    // 6. Test CycloneDX 1.5 SBOM Export
    let cdx = state
        .security
        .export_sbom_cyclonedx("user-dev", "repo-sec")
        .await
        .unwrap();

    assert_eq!(cdx["bomFormat"], "CycloneDX");
    assert_eq!(cdx["specVersion"], "1.5");
    let components = cdx["components"].as_array().expect("components array");
    assert_eq!(components.len(), 2);

    // 7. Test Security Summary
    let summary = state
        .security
        .get_security_summary("user-dev", "repo-sec")
        .await
        .unwrap();

    assert_eq!(summary.total_secrets_found, 1);
    assert_eq!(summary.open_secrets_count, 0); // Already resolved
    assert_eq!(summary.total_dependencies, 2);
    assert_eq!(summary.vulnerable_dependencies_count, 1);
}
