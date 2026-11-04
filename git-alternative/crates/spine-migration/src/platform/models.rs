use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PlatformParseError {
    #[error("I/O error reading platform archive metadata: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON parsing error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("Missing mandatory metadata file: {0}")]
    MissingFile(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformUser {
    pub username: String,
    pub email: Option<String>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformLabel {
    pub name: String,
    pub color: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformMilestone {
    pub title: String,
    pub description: Option<String>,
    pub state: String, // "open" | "closed"
    pub due_on: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformComment {
    pub id: u64,
    pub author: PlatformUser,
    pub body: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformIssue {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String, // "open" | "closed"
    pub author: PlatformUser,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub closed_at: Option<DateTime<Utc>>,
    pub comments: Vec<PlatformComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformPullRequest {
    pub number: u64,
    pub title: String,
    pub body: String,
    pub state: String, // "open" | "closed" | "merged"
    pub author: PlatformUser,
    pub source_branch: String,
    pub target_branch: String,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: Option<DateTime<Utc>>,
    pub merged_at: Option<DateTime<Utc>>,
    pub comments: Vec<PlatformComment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformRelease {
    pub tag_name: String,
    pub name: String,
    pub body: String,
    pub is_prerelease: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PlatformRepositoryData {
    pub name: String,
    pub description: Option<String>,
    pub default_branch: String,
    pub is_private: bool,
    pub topics: Vec<String>,
    pub labels: Vec<PlatformLabel>,
    pub milestones: Vec<PlatformMilestone>,
    pub issues: Vec<PlatformIssue>,
    pub pull_requests: Vec<PlatformPullRequest>,
    pub releases: Vec<PlatformRelease>,
}

/// Parses metadata from an extracted GitHub migration directory.
pub fn parse_github_metadata_dir<P: AsRef<Path>>(
    dir: P,
) -> Result<PlatformRepositoryData, PlatformParseError> {
    let dir = dir.as_ref();
    let mut data = PlatformRepositoryData {
        name: "imported-repo".to_string(),
        default_branch: "main".to_string(),
        ..Default::default()
    };

    // repository.json
    let repo_json = dir.join("repository.json");
    if repo_json.exists() {
        let val: serde_json::Value = serde_json::from_str(&fs::read_to_string(&repo_json)?)?;
        if let Some(n) = val.get("name").and_then(|v| v.as_str()) {
            data.name = n.to_string();
        }
        if let Some(d) = val.get("description").and_then(|v| v.as_str()) {
            data.description = Some(d.to_string());
        }
        if let Some(b) = val.get("default_branch").and_then(|v| v.as_str()) {
            data.default_branch = b.to_string();
        }
        if let Some(p) = val.get("private").and_then(|v| v.as_bool()) {
            data.is_private = p;
        }
        if let Some(topics) = val.get("topics").and_then(|v| v.as_array()) {
            data.topics = topics
                .iter()
                .filter_map(|t| t.as_str().map(String::from))
                .collect();
        }
    }

    // labels.json
    let labels_json = dir.join("labels.json");
    if labels_json.exists() {
        if let Ok(labels) =
            serde_json::from_str::<Vec<PlatformLabel>>(&fs::read_to_string(&labels_json)?)
        {
            data.labels = labels;
        }
    }

    // milestones.json
    let milestones_json = dir.join("milestones.json");
    if milestones_json.exists() {
        if let Ok(milestones) =
            serde_json::from_str::<Vec<PlatformMilestone>>(&fs::read_to_string(&milestones_json)?)
        {
            data.milestones = milestones;
        }
    }

    // issues.json
    let issues_json = dir.join("issues.json");
    if issues_json.exists() {
        if let Ok(issues) =
            serde_json::from_str::<Vec<PlatformIssue>>(&fs::read_to_string(&issues_json)?)
        {
            data.issues = issues;
        }
    }

    // pull_requests.json
    let pulls_json = dir.join("pull_requests.json");
    if pulls_json.exists() {
        if let Ok(pulls) =
            serde_json::from_str::<Vec<PlatformPullRequest>>(&fs::read_to_string(&pulls_json)?)
        {
            data.pull_requests = pulls;
        }
    }

    // releases.json
    let releases_json = dir.join("releases.json");
    if releases_json.exists() {
        if let Ok(releases) =
            serde_json::from_str::<Vec<PlatformRelease>>(&fs::read_to_string(&releases_json)?)
        {
            data.releases = releases;
        }
    }

    Ok(data)
}

/// Parses metadata from an extracted GitLab migration directory.
pub fn parse_gitlab_metadata_dir<P: AsRef<Path>>(
    dir: P,
) -> Result<PlatformRepositoryData, PlatformParseError> {
    let dir = dir.as_ref();
    let mut data = PlatformRepositoryData {
        name: "gitlab-imported-repo".to_string(),
        default_branch: "main".to_string(),
        ..Default::default()
    };

    // project.json
    let project_json = dir.join("project.json");
    if project_json.exists() {
        let val: serde_json::Value = serde_json::from_str(&fs::read_to_string(&project_json)?)?;
        if let Some(n) = val.get("name").and_then(|v| v.as_str()) {
            data.name = n.to_string();
        }
        if let Some(d) = val.get("description").and_then(|v| v.as_str()) {
            data.description = Some(d.to_string());
        }
        if let Some(b) = val.get("default_branch").and_then(|v| v.as_str()) {
            data.default_branch = b.to_string();
        }
        if let Some(vis) = val.get("visibility").and_then(|v| v.as_str()) {
            data.is_private = vis != "public";
        }
    }

    // issues.json
    let issues_json = dir.join("issues.json");
    if issues_json.exists() {
        if let Ok(issues) =
            serde_json::from_str::<Vec<PlatformIssue>>(&fs::read_to_string(&issues_json)?)
        {
            data.issues = issues;
        }
    }

    // merge_requests.json
    let mr_json = dir.join("merge_requests.json");
    if mr_json.exists() {
        if let Ok(pulls) =
            serde_json::from_str::<Vec<PlatformPullRequest>>(&fs::read_to_string(&mr_json)?)
        {
            data.pull_requests = pulls;
        }
    }

    Ok(data)
}
