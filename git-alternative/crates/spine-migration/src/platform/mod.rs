pub mod attribution;
pub mod models;
pub mod sanitization;

pub use attribution::{AttributionPolicy, ResolvedAttribution};
pub use models::{
    parse_github_metadata_dir, parse_gitlab_metadata_dir, PlatformComment, PlatformIssue,
    PlatformLabel, PlatformMilestone, PlatformPullRequest, PlatformRelease, PlatformRepositoryData,
    PlatformUser,
};
pub use sanitization::{is_workflow_file, scan_for_secrets, SanitizationReport, SecretDetection};
