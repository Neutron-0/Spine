//! # spine-migration
//!
//! Native Git interoperability, repository import/export, platform migration (GitHub/GitLab),
//! and ecosystem compatibility for the Spine Version Control System and Spine Forge.

pub mod archive_security;
pub mod engine;
pub mod fast_stream;
pub mod git_exporter;
pub mod git_format;
pub mod git_importer;
pub mod lfs_interop;
pub mod platform;

pub use archive_security::{
    extract_tar_secure, extract_zip_secure, validate_and_sanitize_archive_path,
    ArchiveSecurityError, ExtractionLimits,
};
pub use engine::{
    ExportStatsSummary, ImportStatsSummary, MigrationEngine, MigrationInspection,
    MigrationManifest, MigrationReport, MigrationSourceKind, MigrationStage,
};
pub use fast_stream::{
    parse_fast_import_stream, write_fast_command, FastBlob, FastCommand, FastCommit, FastFileOp,
    FastReset, FastStreamError, FastTag,
};
pub use git_exporter::{ExportStats, GitExportError, GitExporter};
pub use git_format::{
    build_pack_and_index, parse_git_commit, parse_git_tag, parse_git_tree, parse_loose_object,
    parse_packfile, read_git_refs, serialize_git_commit, serialize_git_tag, serialize_git_tree,
    serialize_loose_object, write_git_refs, GitCommit, GitHead, GitObjectType, GitOid, GitRefMap,
    GitTreeEntry,
};
pub use git_importer::{GitImportError, GitImporter, ImportStats};
pub use lfs_interop::{GitLfsPointer, LfsError};
pub use platform::{
    is_workflow_file, parse_github_metadata_dir, parse_gitlab_metadata_dir, scan_for_secrets,
    AttributionPolicy, PlatformIssue, PlatformPullRequest, PlatformRepositoryData,
    ResolvedAttribution, SanitizationReport,
};
