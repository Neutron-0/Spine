use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Row};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LfsPreflightInventory {
    pub total_manifests_inspected: usize,
    pub non_lfs_excluded_count: usize,
    pub already_mapped_count: usize,
    pub ready_to_migrate_count: usize,
    pub invalid_oids: Vec<InvalidOidItem>,
    pub missing_repositories: Vec<MissingRepoItem>,
    pub ambiguous_ownership: Vec<AmbiguousOwnershipItem>,
    pub duplicate_collisions: Vec<DuplicateCollisionItem>,
    pub is_safe_to_migrate: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvalidOidItem {
    pub manifest_id: String,
    pub artifact_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissingRepoItem {
    pub manifest_id: String,
    pub artifact_id: String,
    pub target_repo_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AmbiguousOwnershipItem {
    pub manifest_id: String,
    pub artifact_id: String,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DuplicateCollisionItem {
    pub repository_id: String,
    pub oid: String,
    pub manifest_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperatorLfsMapping {
    pub manifest_id: String,
    pub repository_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemediationResult {
    pub is_dry_run: bool,
    pub mappings_validated: usize,
    pub manifests_remediated: usize,
    pub already_correct: usize,
}

/// Read-only inventory of legacy artifact manifests to preflight migration 0016.
pub async fn inventory_legacy_lfs_manifests(
    pool: &PgPool,
) -> Result<LfsPreflightInventory, anyhow::Error> {
    // Check if lfs_objects table exists
    let lfs_table_exists: (bool,) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM information_schema.tables WHERE table_name = 'lfs_objects')",
    )
    .fetch_one(pool)
    .await?;

    // Check existing forge_repositories
    let repo_rows = sqlx::query("SELECT id FROM forge_repositories")
        .fetch_all(pool)
        .await?;
    let known_repos: HashSet<String> = repo_rows.iter().map(|r| r.get::<String, _>("id")).collect();

    // Check non-LFS manifests
    let non_lfs_rows = sqlx::query(
        "SELECT manifest_id FROM release_assets
         UNION
         SELECT manifest_id FROM package_versions
         UNION
         SELECT manifest_id FROM artifact_versions",
    )
    .fetch_all(pool)
    .await?;
    let non_lfs_manifests: HashSet<String> = non_lfs_rows
        .iter()
        .map(|r| r.get::<String, _>("manifest_id"))
        .collect();

    // Check already-mapped manifests if lfs_objects exists
    let mapped_manifests: HashSet<String> = if lfs_table_exists.0 {
        let mapped_rows = sqlx::query("SELECT manifest_id FROM lfs_objects")
            .fetch_all(pool)
            .await?;
        mapped_rows
            .iter()
            .map(|r| r.get::<String, _>("manifest_id"))
            .collect()
    } else {
        HashSet::new()
    };

    // Query all artifact_manifests
    let all_manifests = sqlx::query(
        "SELECT id, artifact_id, total_size, manifest_json, created_at_nanos FROM artifact_manifests ORDER BY created_at_nanos ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut non_lfs_excluded_count = 0;
    let mut already_mapped_count = 0;
    let mut ready_to_migrate_count = 0;
    let mut invalid_oids = Vec::new();
    let mut missing_repositories = Vec::new();
    let mut ambiguous_ownership = Vec::new();

    // Map of (repo_id, oid) -> Vec<manifest_id> for collision checking
    let mut candidate_repo_oids: HashMap<(String, String), Vec<String>> = HashMap::new();

    for m in &all_manifests {
        let id: String = m.get("id");
        let artifact_id: String = m.get("artifact_id");
        let manifest_json_str: String = m.get("manifest_json");

        // 1. Is it a non-LFS manifest?
        if non_lfs_manifests.contains(&id) {
            non_lfs_excluded_count += 1;
            continue;
        }

        // 2. Is it already mapped in lfs_objects?
        if mapped_manifests.contains(&id) {
            already_mapped_count += 1;
            continue;
        }

        // Parse JSON
        let parsed_json: serde_json::Value =
            serde_json::from_str(&manifest_json_str).unwrap_or(serde_json::Value::Null);

        let has_repo_metadata =
            parsed_json.get("repository_id").is_some() || parsed_json.get("repo_id").is_some();

        let is_potential_lfs = (artifact_id.len() == 64
            && artifact_id.chars().all(|c| c.is_ascii_hexdigit()))
            || has_repo_metadata;

        if !is_potential_lfs {
            // Neither 64-hex SHA-256 nor repo metadata -> not LFS
            non_lfs_excluded_count += 1;
            continue;
        }

        // 3. Validate OID format (must be 64-character lowercase hex)
        if artifact_id.len() != 64 || !artifact_id.chars().all(|c| c.is_ascii_hexdigit()) {
            invalid_oids.push(InvalidOidItem {
                manifest_id: id.clone(),
                artifact_id: artifact_id.clone(),
                reason: "artifact_id is not a valid 64-hex SHA-256 OID".to_string(),
            });
            continue;
        }

        // 4. Resolve repository_id
        let target_repo = parsed_json
            .get("repository_id")
            .and_then(|v| v.as_str())
            .or_else(|| parsed_json.get("repo_id").and_then(|v| v.as_str()));

        match target_repo {
            None => {
                ambiguous_ownership.push(AmbiguousOwnershipItem {
                    manifest_id: id.clone(),
                    artifact_id: artifact_id.clone(),
                    reason: "manifest lacks explicit repository_id metadata in manifest_json"
                        .to_string(),
                });
            }
            Some(repo_id) => {
                let repo_id_trimmed = repo_id.trim();
                if repo_id_trimmed.is_empty() {
                    ambiguous_ownership.push(AmbiguousOwnershipItem {
                        manifest_id: id.clone(),
                        artifact_id: artifact_id.clone(),
                        reason: "repository_id metadata is empty string".to_string(),
                    });
                } else if !known_repos.contains(repo_id_trimmed) {
                    missing_repositories.push(MissingRepoItem {
                        manifest_id: id.clone(),
                        artifact_id: artifact_id.clone(),
                        target_repo_id: repo_id_trimmed.to_string(),
                    });
                } else {
                    candidate_repo_oids
                        .entry((repo_id_trimmed.to_string(), artifact_id.to_lowercase()))
                        .or_default()
                        .push(id.clone());
                }
            }
        }
    }

    // 5. Detect duplicate collisions
    let mut duplicate_collisions = Vec::new();
    for ((repo_id, oid), manifest_ids) in candidate_repo_oids {
        if manifest_ids.len() > 1 {
            duplicate_collisions.push(DuplicateCollisionItem {
                repository_id: repo_id,
                oid,
                manifest_ids,
            });
        } else {
            ready_to_migrate_count += 1;
        }
    }

    let is_safe_to_migrate = invalid_oids.is_empty()
        && missing_repositories.is_empty()
        && ambiguous_ownership.is_empty()
        && duplicate_collisions.is_empty();

    Ok(LfsPreflightInventory {
        total_manifests_inspected: all_manifests.len(),
        non_lfs_excluded_count,
        already_mapped_count,
        ready_to_migrate_count,
        invalid_oids,
        missing_repositories,
        ambiguous_ownership,
        duplicate_collisions,
        is_safe_to_migrate,
    })
}

/// Safely remediate unmapped legacy LFS manifests using explicit operator-provided mappings.
/// NEVER infers ownership.
/// Supports dry-run, atomic transaction rollback on failure, and idempotent rerun.
pub async fn remediate_legacy_lfs_manifests(
    pool: &PgPool,
    mappings: &[OperatorLfsMapping],
    dry_run: bool,
) -> Result<RemediationResult, anyhow::Error> {
    if mappings.is_empty() {
        anyhow::bail!("Operator mappings list is empty. Explicit mappings are required.");
    }

    // Check for duplicate manifest IDs in input
    let mut seen_input_manifests = HashSet::new();
    for m in mappings {
        if !seen_input_manifests.insert(&m.manifest_id) {
            anyhow::bail!(
                "Malformed operator mapping input: duplicate manifest_id '{}' supplied",
                m.manifest_id
            );
        }
    }

    let mut tx = pool.begin().await?;

    let mut mappings_validated = 0;
    let mut manifests_remediated = 0;
    let mut already_correct = 0;

    for m in mappings {
        let manifest_id = m.manifest_id.trim();
        let repo_id = m.repository_id.trim();

        if manifest_id.is_empty() {
            anyhow::bail!("Malformed operator mapping: manifest_id cannot be blank");
        }
        if repo_id.is_empty() {
            anyhow::bail!("Malformed operator mapping: repository_id cannot be blank");
        }

        // 1. Fetch manifest row
        let row_opt =
            sqlx::query("SELECT artifact_id, manifest_json FROM artifact_manifests WHERE id = $1")
                .bind(manifest_id)
                .fetch_optional(&mut *tx)
                .await?;

        let row = match row_opt {
            Some(r) => r,
            None => {
                anyhow::bail!(
                    "Remediation rejected: manifest '{}' does not exist in artifact_manifests",
                    manifest_id
                );
            }
        };

        let artifact_id: String = row.get("artifact_id");
        let manifest_json_str: String = row.get("manifest_json");

        // 2. Validate artifact_id is valid SHA-256 OID
        if artifact_id.len() != 64 || !artifact_id.chars().all(|c| c.is_ascii_hexdigit()) {
            anyhow::bail!(
                "Remediation rejected: manifest '{}' has invalid artifact_id '{}' (not a 64-hex SHA-256 OID)",
                manifest_id,
                artifact_id
            );
        }

        // 3. Confirm not non-LFS artifact
        let is_non_lfs: (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                SELECT 1 FROM release_assets WHERE manifest_id = $1
                UNION
                SELECT 1 FROM package_versions WHERE manifest_id = $1
                UNION
                SELECT 1 FROM artifact_versions WHERE manifest_id = $1
            )",
        )
        .bind(manifest_id)
        .fetch_one(&mut *tx)
        .await?;

        if is_non_lfs.0 {
            anyhow::bail!(
                "Remediation rejected: manifest '{}' is a non-LFS artifact (release asset, package, or artifact version)",
                manifest_id
            );
        }

        // 4. Confirm target repository exists in forge_repositories
        let repo_exists: (bool,) =
            sqlx::query_as("SELECT EXISTS (SELECT 1 FROM forge_repositories WHERE id = $1)")
                .bind(repo_id)
                .fetch_one(&mut *tx)
                .await?;

        if !repo_exists.0 {
            anyhow::bail!(
                "Remediation rejected: target repository '{}' does not exist in forge_repositories",
                repo_id
            );
        }

        // 5. Check if already correct
        let parsed: serde_json::Value =
            serde_json::from_str(&manifest_json_str).unwrap_or(serde_json::Value::Null);
        let existing_repo = parsed.get("repository_id").and_then(|v| v.as_str());

        if existing_repo == Some(repo_id) {
            already_correct += 1;
            mappings_validated += 1;
            continue;
        }

        // 6. Check for duplicate collision with existing assigned manifests
        let conflict: (bool,) = sqlx::query_as(
            "SELECT EXISTS (
                SELECT 1 FROM artifact_manifests
                WHERE id != $1
                  AND lower(artifact_id) = lower($2)
                  AND (manifest_json::jsonb->>'repository_id' = $3 OR manifest_json::jsonb->>'repo_id' = $3)
            )",
        )
        .bind(manifest_id)
        .bind(&artifact_id)
        .bind(repo_id)
        .fetch_one(&mut *tx)
        .await?;

        if conflict.0 {
            anyhow::bail!(
                "Remediation rejected: duplicate collision for repository '{}' and OID '{}'",
                repo_id,
                artifact_id
            );
        }

        // 7. Update manifest_json with explicit repository_id
        sqlx::query(
            "UPDATE artifact_manifests
             SET manifest_json = jsonb_set(
                 manifest_json::jsonb,
                 '{repository_id}',
                 to_jsonb($2::text)
             )::text
             WHERE id = $1",
        )
        .bind(manifest_id)
        .bind(repo_id)
        .execute(&mut *tx)
        .await?;

        manifests_remediated += 1;
        mappings_validated += 1;
    }

    if dry_run {
        tx.rollback().await?;
    } else {
        tx.commit().await?;
    }

    Ok(RemediationResult {
        is_dry_run: dry_run,
        mappings_validated,
        manifests_remediated,
        already_correct,
    })
}
