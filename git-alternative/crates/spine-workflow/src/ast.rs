use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub name: String,
    pub on: TriggerSpec,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub jobs: HashMap<String, JobSpec>,
}

impl WorkflowSpec {
    pub fn digest(&self) -> String {
        let json = serde_json::to_vec(self).unwrap_or_default();
        blake3::hash(&json).to_hex().to_string()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriggerSpec {
    pub push: Option<EventFilter>,
    pub pull_request: Option<EventFilter>,
    pub workflow_dispatch: Option<bool>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EventFilter {
    #[serde(default)]
    pub branches: Vec<String>,
    #[serde(default)]
    pub paths: Vec<String>,
}

impl EventFilter {
    pub fn matches_branch(&self, branch: &str) -> bool {
        if self.branches.is_empty() {
            return true;
        }
        for pat in &self.branches {
            if pat == "*" || pat == branch {
                return true;
            }
            if let Some(prefix) = pat.strip_suffix("/*") {
                if branch.starts_with(prefix) {
                    return true;
                }
            }
        }
        false
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobSpec {
    pub name: Option<String>,
    #[serde(default)]
    pub runs_on: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    pub timeout_minutes: Option<u32>,
    pub continue_on_error: Option<bool>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub steps: Vec<StepSpec>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepSpec {
    pub id: Option<String>,
    pub name: Option<String>,
    pub run: Option<String>,
    pub uses: Option<String>,
    #[serde(default)]
    pub with: HashMap<String, String>,
    pub working_directory: Option<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
    pub timeout_minutes: Option<u32>,
}
