use crate::api::middleware::AppState;
use crate::core::error::ForgeError;
use crate::core::types::User;
use axum::{
    extract::{Extension, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::{Deserialize, Serialize};
use spine_migration::engine::MigrationEngine;
use spine_migration::platform::attribution::AttributionPolicy;

#[derive(Debug, Deserialize)]
pub struct InspectRequest {
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
pub struct PlanRequest {
    pub source_path: String,
}

#[derive(Debug, Deserialize)]
pub struct RunMigrationRequest {
    pub source_path: String,
    pub target_name: String,
    #[serde(default)]
    pub dry_run: bool,
}

#[derive(Debug, Deserialize)]
pub struct ExportGitRequest {
    pub repo_name: String,
    pub target_path: String,
    #[serde(default)]
    pub is_bare: bool,
}

#[derive(Debug, Serialize)]
pub struct MigrationPlanResponse {
    pub source_path: String,
    pub detected_kind: String,
    pub stages: Vec<String>,
    pub estimated_blobs: usize,
    pub estimated_commits: usize,
}

pub fn get_approved_migration_root(state: &AppState) -> std::path::PathBuf {
    if let Ok(root_str) = std::env::var("SPINE_MIGRATION_ROOT") {
        let p = std::path::PathBuf::from(root_str);
        if p.is_absolute() {
            let _ = std::fs::create_dir_all(&p);
            return p;
        }
    }
    let p = state.storage_root().join("migrations");
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn resolve_and_validate_migration_path(
    raw_path: &str,
    approved_root: &std::path::Path,
) -> Result<std::path::PathBuf, (StatusCode, String)> {
    let p = std::path::Path::new(raw_path);
    if p.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
        return Err((
            StatusCode::BAD_REQUEST,
            "Path traversal ('..') is strictly prohibited in migration paths".to_string(),
        ));
    }

    // Resolve relative to approved root if relative, or use directly if absolute
    let target = if p.is_relative() {
        approved_root.join(p)
    } else {
        p.to_path_buf()
    };

    // Canonicalize approved root
    let approved_canonical = match approved_root.canonicalize() {
        Ok(c) => c,
        Err(_) => approved_root.to_path_buf(),
    };

    // Normalize target path
    let target_canonical = match target.canonicalize() {
        Ok(c) => c,
        Err(_) => {
            if let Some(parent) = target.parent() {
                if let Ok(canon_parent) = parent.canonicalize() {
                    canon_parent.join(target.file_name().unwrap_or_default())
                } else {
                    target.clone()
                }
            } else {
                target.clone()
            }
        }
    };

    let approved_str = approved_canonical.to_string_lossy().replace(r"\\?\", "");
    let target_str = target_canonical.to_string_lossy().replace(r"\\?\", "");

    let approved_normalized = std::path::Path::new(&approved_str);
    let target_normalized = std::path::Path::new(&target_str);

    if !target_normalized.starts_with(approved_normalized) {
        return Err((
            StatusCode::FORBIDDEN,
            format!(
                "Migration path '{}' is outside approved migration root '{}'",
                raw_path,
                approved_normalized.display()
            ),
        ));
    }

    Ok(target)
}

fn validate_target_repo_name(name: &str) -> Result<(), (StatusCode, String)> {
    if name.trim().is_empty()
        || name.contains('/')
        || name.contains('\\')
        || name.contains("..")
        || name.len() > 100
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid repository name for migration target".to_string(),
        ));
    }
    Ok(())
}

fn validate_export_repo_name(name: &str) -> Result<(), (StatusCode, String)> {
    if name.trim().is_empty()
        || name.contains('\\')
        || name.contains("..")
        || name.len() > 100
    {
        return Err((
            StatusCode::BAD_REQUEST,
            "Invalid repository name for export".to_string(),
        ));
    }
    Ok(())
}

pub async fn inspect_source(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(req): Json<InspectRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let _caller = user.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Authentication required for migration operations".to_string(),
        )
    })?;
    let approved_root = get_approved_migration_root(&state);
    let validated_source = resolve_and_validate_migration_path(&req.source_path, &approved_root)?;

    let inspection = MigrationEngine::inspect(&validated_source)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Inspection error: {}", e)))?;
    Ok(Json(inspection))
}

pub async fn plan_migration(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(req): Json<PlanRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let _caller = user.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Authentication required for migration operations".to_string(),
        )
    })?;
    let approved_root = get_approved_migration_root(&state);
    let validated_source = resolve_and_validate_migration_path(&req.source_path, &approved_root)?;

    let inspection = MigrationEngine::inspect(&validated_source)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("Planning error: {}", e)))?;

    let plan = MigrationPlanResponse {
        source_path: req.source_path,
        detected_kind: format!("{:?}", inspection.source_kind),
        stages: vec![
            "Security and path traversal pre-flight checks".to_string(),
            "Blob and LFS artifact ingestion into CAS".to_string(),
            "Tree reconstruction with canonical sorting".to_string(),
            "Commit DAG ordering and author normalization".to_string(),
            "Branch and tag reference synchronization".to_string(),
            "Platform metadata hydration with safe attribution".to_string(),
            "Integrity verification & cryptographic audit".to_string(),
        ],
        estimated_blobs: inspection.commit_count * 5 + 10,
        estimated_commits: inspection.commit_count,
    };

    Ok(Json(plan))
}

pub async fn run_migration(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(req): Json<RunMigrationRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let caller = user.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Authentication required for migration operations".to_string(),
        )
    })?
    .0;
    validate_target_repo_name(&req.target_name)?;

    let approved_root = get_approved_migration_root(&state);
    let validated_source = resolve_and_validate_migration_path(&req.source_path, &approved_root)?;

    let storage_root = state.storage_root();
    let native_repo_id = format!("{}_{}", caller.username.to_lowercase(), req.target_name.to_lowercase());
    let target_dir = storage_root.join("repos").join(&native_repo_id);

    let policy = AttributionPolicy {
        fallback_spine_user: caller.username,
        ..Default::default()
    };

    let report =
        MigrationEngine::run_git_import(&validated_source, &target_dir, policy, req.dry_run)
            .map_err(|e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    format!("Migration error: {}", e),
                )
            })?;

    Ok(Json(report))
}

pub async fn export_git(
    State(state): State<AppState>,
    user: Option<Extension<User>>,
    Json(req): Json<ExportGitRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let caller = user.ok_or_else(|| {
        (
            StatusCode::UNAUTHORIZED,
            "Authentication required for migration operations".to_string(),
        )
    })?
    .0;
    validate_export_repo_name(&req.repo_name)?;

    let (owner, name) = if let Some((o, n)) = req.repo_name.split_once('/') {
        (o.to_string(), n.to_string())
    } else {
        (caller.username.clone(), req.repo_name.clone())
    };

    let repo = state
        .repositories
        .get_repository(Some(&caller), &owner, &name)
        .await
        .map_err(|e| match e {
            ForgeError::NotFound(_) => {
                (StatusCode::NOT_FOUND, format!("Repository '{}' not found", req.repo_name))
            }
            ForgeError::Forbidden(_) => {
                (StatusCode::FORBIDDEN, "Forbidden: write access required to export".to_string())
            }
            ForgeError::Unauthorized(_) => {
                (StatusCode::UNAUTHORIZED, "Authentication required".to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    state
        .permissions
        .check_write_access(Some(&caller), &repo)
        .await
        .map_err(|e| match e {
            ForgeError::NotFound(_) => {
                (StatusCode::NOT_FOUND, format!("Repository '{}' not found", req.repo_name))
            }
            ForgeError::Forbidden(_) => {
                (StatusCode::FORBIDDEN, "Forbidden: write access required to export".to_string())
            }
            ForgeError::Unauthorized(_) => {
                (StatusCode::UNAUTHORIZED, "Authentication required".to_string())
            }
            _ => (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
        })?;

    let approved_root = get_approved_migration_root(&state);
    let validated_target = resolve_and_validate_migration_path(&req.target_path, &approved_root)?;

    let storage_root = state.storage_root();
    let source_spine_repo = storage_root.join("repos").join(&repo.native_repo_id);

    if !source_spine_repo.exists() {
        return Err((
            StatusCode::NOT_FOUND,
            format!("Repository data for '{}' not found on storage", req.repo_name),
        ));
    }

    let stats = MigrationEngine::run_git_export(
        &source_spine_repo,
        &validated_target,
        req.is_bare,
    )
    .map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            format!("Git export error: {}", e),
        )
    })?;

    Ok(Json(stats))
}
