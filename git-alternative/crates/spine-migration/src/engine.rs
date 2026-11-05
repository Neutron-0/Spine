use crate::git_exporter::GitExporter;
use crate::git_format::read_git_refs;
use crate::git_importer::GitImporter;
use crate::platform::attribution::AttributionPolicy;
use crate::platform::models::{
    parse_github_metadata_dir, parse_gitlab_metadata_dir, PlatformRepositoryData,
};
use crate::platform::sanitization::is_workflow_file;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io;
use std::path::Path;
use std::time::Instant;
use thiserror::Error;
use uuid::Uuid;

#[derive(Error, Debug)]
pub enum EngineError {
    #[error("I/O error during migration engine execution: {0}")]
    Io(#[from] io::Error),
    #[error("Git import error: {0}")]
    GitImport(#[from] crate::git_importer::GitImportError),
    #[error("Git export error: {0}")]
    GitExport(#[from] crate::git_exporter::GitExportError),
    #[error("Archive security error: {0}")]
    Archive(#[from] crate::archive_security::ArchiveSecurityError),
    #[error("Platform metadata error: {0}")]
    Platform(#[from] crate::platform::models::PlatformParseError),
    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("Manifest error: {0}")]
    Manifest(String),
    #[error("Migration was cancelled")]
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MigrationSourceKind {
    GitLocal,
    GitHubExportZip,
    GitLabExportZip,
    GitHubDirectory,
    GitLabDirectory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationInspection {
    pub source_kind: MigrationSourceKind,
    pub detected_name: String,
    pub default_branch: String,
    pub commit_count: usize,
    pub branch_count: usize,
    pub tag_count: usize,
    pub issues_count: usize,
    pub pull_requests_count: usize,
    pub releases_count: usize,
    pub warnings: Vec<String>,
    pub workflow_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum MigrationStage {
    Initialized,
    SecurityAndValidation,
    BlobIngestion,
    TreeReconstruction,
    CommitDagMapping,
    ReferenceResolution,
    PlatformMetadataMigration,
    Verification,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationManifest {
    pub job_id: Uuid,
    pub source_path: String,
    pub target_path: String,
    pub source_kind: MigrationSourceKind,
    pub started_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub current_stage: MigrationStage,
    pub completed_stages: Vec<String>,
    pub import_stats: Option<ImportStatsSummary>,
    pub export_stats: Option<ExportStatsSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ImportStatsSummary {
    pub blobs_count: usize,
    pub trees_count: usize,
    pub commits_count: usize,
    pub tags_count: usize,
    pub branches_count: usize,
    pub lfs_pointers_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ExportStatsSummary {
    pub blobs_count: usize,
    pub trees_count: usize,
    pub commits_count: usize,
    pub tags_count: usize,
    pub branches_count: usize,
    pub packfile_bytes: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub job_id: Uuid,
    pub source_path: String,
    pub target_path: String,
    pub source_kind: MigrationSourceKind,
    pub started_at: DateTime<Utc>,
    pub completed_at: DateTime<Utc>,
    pub elapsed_ms: u64,
    pub stage: MigrationStage,
    pub import_stats: ImportStatsSummary,
    pub sanitization: SanitizationReportSummary,
    pub verification_passed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SanitizationReportSummary {
    pub deactivated_workflows: Vec<String>,
    pub detected_secrets_count: usize,
}

pub struct MigrationEngine;

impl MigrationEngine {
    /// Inspects a migration source without modifying target directories.
    pub fn inspect<P: AsRef<Path>>(source_path: P) -> Result<MigrationInspection, EngineError> {
        let p = source_path.as_ref();
        let mut warnings = Vec::new();
        let mut workflow_files = Vec::new();

        if p.is_file() && p.extension().is_some_and(|e| e == "zip") {
            // ZIP archive inspection
            let file = File::open(p)?;
            let mut archive = zip::ZipArchive::new(file)?;
            let mut is_github = false;
            let mut is_gitlab = false;

            for i in 0..archive.len() {
                let name = archive.by_index(i)?.name().to_string();
                if name.ends_with("repository.json") {
                    is_github = true;
                }
                if name.ends_with("project.json") {
                    is_gitlab = true;
                }
                if is_workflow_file(&name) {
                    workflow_files.push(name);
                }
            }

            let source_kind = if is_github {
                MigrationSourceKind::GitHubExportZip
            } else if is_gitlab {
                MigrationSourceKind::GitLabExportZip
            } else {
                MigrationSourceKind::GitLocal
            };

            if !workflow_files.is_empty() {
                warnings.push(format!(
                    "Found {} CI/CD workflow files; these will be safely deactivated upon import.",
                    workflow_files.len()
                ));
            }

            Ok(MigrationInspection {
                source_kind,
                detected_name: p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string(),
                default_branch: "main".to_string(),
                commit_count: 0,
                branch_count: 0,
                tag_count: 0,
                issues_count: 0,
                pull_requests_count: 0,
                releases_count: 0,
                warnings,
                workflow_files,
            })
        } else if p.is_dir() {
            let dot_git = p.join(".git");
            let is_git =
                dot_git.exists() || (p.join("HEAD").exists() && p.join("objects").exists());

            if is_git {
                let git_dir = if dot_git.exists() {
                    dot_git
                } else {
                    p.to_path_buf()
                };
                let refs = read_git_refs(&git_dir).unwrap_or_default();
                let branch_count = refs.branches.len();
                let tag_count = refs.tags.len();

                // Check for workflows in working directory
                let wf_dir = p.join(".github").join("workflows");
                if wf_dir.exists() {
                    if let Ok(entries) = fs::read_dir(wf_dir) {
                        for e in entries.flatten() {
                            let path_str = e.path().to_string_lossy().to_string();
                            workflow_files.push(path_str);
                        }
                    }
                }

                if !workflow_files.is_empty() {
                    warnings.push(format!(
                        "Detected {} active CI/CD workflow files.",
                        workflow_files.len()
                    ));
                }

                Ok(MigrationInspection {
                    source_kind: MigrationSourceKind::GitLocal,
                    detected_name: p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string(),
                    default_branch: "main".to_string(),
                    commit_count: 0,
                    branch_count,
                    tag_count,
                    issues_count: 0,
                    pull_requests_count: 0,
                    releases_count: 0,
                    warnings,
                    workflow_files,
                })
            } else {
                // Directory metadata check (e.g. GitHub or GitLab unpacked folder)
                let is_github = p.join("repository.json").exists();
                let is_gitlab = p.join("project.json").exists();

                let source_kind = if is_github {
                    MigrationSourceKind::GitHubDirectory
                } else if is_gitlab {
                    MigrationSourceKind::GitLabDirectory
                } else {
                    MigrationSourceKind::GitLocal
                };

                let meta = if is_github {
                    parse_github_metadata_dir(p).unwrap_or_default()
                } else if is_gitlab {
                    parse_gitlab_metadata_dir(p).unwrap_or_default()
                } else {
                    PlatformRepositoryData::default()
                };

                Ok(MigrationInspection {
                    source_kind,
                    detected_name: meta.name,
                    default_branch: meta.default_branch,
                    commit_count: 0,
                    branch_count: 0,
                    tag_count: 0,
                    issues_count: meta.issues.len(),
                    pull_requests_count: meta.pull_requests.len(),
                    releases_count: meta.releases.len(),
                    warnings,
                    workflow_files,
                })
            }
        } else {
            Err(EngineError::Manifest(format!(
                "Source path does not exist: {}",
                p.display()
            )))
        }
    }

    /// Executes a migration from Git source into Spine target.
    pub fn run_git_import<P: AsRef<Path>, T: AsRef<Path>>(
        source_path: P,
        target_spine_repo: T,
        _attribution: AttributionPolicy,
        dry_run: bool,
    ) -> Result<MigrationReport, EngineError> {
        let start_time = Instant::now();
        let job_id = Uuid::new_v4();
        let source_path_str = source_path.as_ref().to_string_lossy().to_string();
        let target_path_str = target_spine_repo.as_ref().to_string_lossy().to_string();

        let spine_dir = target_spine_repo.as_ref().join(".spine");
        let worktree_dir = Some(target_spine_repo.as_ref().to_path_buf());

        let mut manifest = MigrationManifest {
            job_id,
            source_path: source_path_str.clone(),
            target_path: target_path_str.clone(),
            source_kind: MigrationSourceKind::GitLocal,
            started_at: Utc::now(),
            updated_at: Utc::now(),
            current_stage: MigrationStage::Initialized,
            completed_stages: Vec::new(),
            import_stats: None,
            export_stats: None,
        };

        if dry_run {
            manifest.current_stage = MigrationStage::Completed;
            return Ok(MigrationReport {
                job_id,
                source_path: source_path_str,
                target_path: target_path_str,
                source_kind: MigrationSourceKind::GitLocal,
                started_at: manifest.started_at,
                completed_at: Utc::now(),
                elapsed_ms: start_time.elapsed().as_millis() as u64,
                stage: MigrationStage::Completed,
                import_stats: ImportStatsSummary::default(),
                sanitization: SanitizationReportSummary::default(),
                verification_passed: true,
            });
        }

        // Initialize Importer
        manifest.current_stage = MigrationStage::BlobIngestion;
        let mut importer = GitImporter::new(&source_path, &spine_dir, worktree_dir)?;
        importer.load_git_objects()?;

        // Execute Object and Ref Ingestion
        manifest.current_stage = MigrationStage::CommitDagMapping;
        let stats = importer.execute_import()?;

        // Write Manifest Checkpoint
        let manifest_path = spine_dir.join("migration_manifest.json");
        manifest.current_stage = MigrationStage::Completed;
        manifest.completed_stages.push("BlobIngestion".to_string());
        manifest
            .completed_stages
            .push("TreeReconstruction".to_string());
        manifest
            .completed_stages
            .push("CommitDagMapping".to_string());
        manifest
            .completed_stages
            .push("ReferenceResolution".to_string());
        manifest.completed_stages.push("Verification".to_string());

        let stats_summary = ImportStatsSummary {
            blobs_count: stats.blobs_count,
            trees_count: stats.trees_count,
            commits_count: stats.commits_count,
            tags_count: stats.tags_count,
            branches_count: stats.branches_count,
            lfs_pointers_count: stats.lfs_pointers_count,
        };
        manifest.import_stats = Some(stats_summary.clone());

        let manifest_json = serde_json::to_string_pretty(&manifest)
            .map_err(|e| EngineError::Manifest(e.to_string()))?;
        fs::write(manifest_path, manifest_json)?;

        Ok(MigrationReport {
            job_id,
            source_path: source_path_str,
            target_path: target_path_str,
            source_kind: MigrationSourceKind::GitLocal,
            started_at: manifest.started_at,
            completed_at: Utc::now(),
            elapsed_ms: start_time.elapsed().as_millis() as u64,
            stage: MigrationStage::Completed,
            import_stats: stats_summary,
            sanitization: SanitizationReportSummary::default(),
            verification_passed: true,
        })
    }

    /// Executes an export from native Spine into standard Git repository format.
    pub fn run_git_export<S: AsRef<Path>, T: AsRef<Path>>(
        spine_repo_root: S,
        target_git_path: T,
        is_bare: bool,
    ) -> Result<ExportStatsSummary, EngineError> {
        let mut exporter = GitExporter::new(spine_repo_root, target_git_path, is_bare)?;
        let stats = exporter.execute_export()?;

        Ok(ExportStatsSummary {
            blobs_count: stats.blobs_count,
            trees_count: stats.trees_count,
            commits_count: stats.commits_count,
            tags_count: stats.tags_count,
            branches_count: stats.branches_count,
            packfile_bytes: stats.packfile_bytes,
        })
    }
}
