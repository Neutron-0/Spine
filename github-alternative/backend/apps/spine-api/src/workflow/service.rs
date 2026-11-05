use chrono::Utc;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use uuid::Uuid;

use spine_workflow::dag::JobDag;
use spine_workflow::parser::WorkflowParser;

use crate::core::error::ForgeError;
use crate::core::types::{
    WorkflowCheckResult, WorkflowDefinition, WorkflowJob, WorkflowRun, WorkflowStep,
};
use crate::db::Database;
use crate::webhook::WebhookService;

use crate::core::profile::DeploymentProfile;

#[derive(Clone)]
pub struct WorkflowService {
    db: Arc<dyn Database>,
    webhook_service: Option<WebhookService>,
    pub profile: DeploymentProfile,
}

impl WorkflowService {
    pub fn new(db: Arc<dyn Database>, webhook_service: Option<WebhookService>) -> Self {
        Self {
            db,
            webhook_service,
            profile: DeploymentProfile::CoreForge,
        }
    }

    pub fn with_profile(mut self, profile: DeploymentProfile) -> Self {
        self.profile = profile;
        self
    }

    /// Sync and validate workflow definitions from repository files (e.g. `.spine/workflows/*.yml`)
    pub async fn sync_workflows_from_tree(
        &self,
        repo_id: &str,
        workflows: Vec<(&str, &str)>, // (path, yaml_content)
    ) -> Result<Vec<WorkflowDefinition>, ForgeError> {
        let mut results = Vec::new();
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        for (path, content) in workflows {
            // Validate syntax and DAG structure
            let spec = match WorkflowParser::parse(content) {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("Skipping invalid workflow {}: {}", path, e);
                    continue;
                }
            };

            if let Err(e) = JobDag::build(&spec) {
                tracing::warn!("Skipping cyclical or malformed workflow {}: {}", path, e);
                continue;
            }

            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            let digest = hex::encode(hasher.finalize());

            let def = WorkflowDefinition {
                id: Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                path: path.to_string(),
                name: spec.name.clone(),
                yaml_content: content.to_string(),
                digest,
                created_at_nanos: now_nanos,
                updated_at_nanos: now_nanos,
            };

            let upserted = self.db.upsert_workflow_definition(def).await?;
            results.push(upserted);
        }

        Ok(results)
    }

    /// Trigger workflows in response to a VCS or Forge event (e.g. "push", "pull_request", "workflow_dispatch")
    pub async fn trigger_workflows(
        &self,
        repo_id: &str,
        commit_oid: &str,
        ref_name: &str,
        event_type: &str,
        actor_id: &str,
    ) -> Result<Vec<WorkflowRun>, ForgeError> {
        let definitions = self.db.list_workflow_definitions(repo_id).await?;
        let mut triggered_runs = Vec::new();
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);

        for def in definitions {
            let spec = match WorkflowParser::parse(&def.yaml_content) {
                Ok(s) => s,
                Err(_) => continue,
            };

            // Evaluate triggers
            let branch_clean = ref_name.strip_prefix("refs/heads/").unwrap_or(ref_name);
            let matched = match event_type {
                "push" => spec
                    .on
                    .push
                    .as_ref()
                    .is_some_and(|p| p.matches_branch(branch_clean)),
                "pull_request" => spec
                    .on
                    .pull_request
                    .as_ref()
                    .is_some_and(|pr| pr.matches_branch(branch_clean)),
                "workflow_dispatch" => spec.on.workflow_dispatch.unwrap_or(true),
                _ => false,
            };

            if !matched {
                continue;
            }

            let is_untrusted_event = event_type == "pull_request_fork"
                || event_type == "fork"
                || event_type.starts_with("untrusted");

            if is_untrusted_event && !self.profile.allow_untrusted_ci() {
                tracing::warn!(
                    "Untrusted CI execution is disabled under deployment profile '{}'. Disallowing workflow run for repo {}",
                    self.profile.name(),
                    repo_id
                );

                let run_id = Uuid::new_v4().to_string();
                let run = WorkflowRun {
                    id: run_id.clone(),
                    repo_id: repo_id.to_string(),
                    workflow_id: def.id.clone(),
                    workflow_name: spec.name.clone(),
                    commit_oid: commit_oid.to_string(),
                    ref_name: ref_name.to_string(),
                    trigger_event: event_type.to_string(),
                    actor_id: actor_id.to_string(),
                    status: "COMPLETED".to_string(),
                    conclusion: Some("DISALLOWED".to_string()),
                    definition_yaml: def.yaml_content.clone(),
                    definition_digest: def.digest.clone(),
                    created_at_nanos: now_nanos,
                    started_at_nanos: Some(now_nanos),
                    completed_at_nanos: Some(now_nanos),
                };

                let created_run = self.db.create_workflow_run(run).await?;

                let check_result = WorkflowCheckResult {
                    id: Uuid::new_v4().to_string(),
                    repo_id: repo_id.to_string(),
                    commit_oid: commit_oid.to_string(),
                    check_name: spec.name.clone(),
                    status: "COMPLETED".to_string(),
                    run_id: run_id.clone(),
                    updated_at_nanos: now_nanos,
                };
                let _ = self.db.upsert_workflow_check_result(check_result).await;

                triggered_runs.push(created_run);
                continue; // Disallowed: no jobs are created or queued to runners
            }

            let run_id = Uuid::new_v4().to_string();
            let run = WorkflowRun {
                id: run_id.clone(),
                repo_id: repo_id.to_string(),
                workflow_id: def.id.clone(),
                workflow_name: spec.name.clone(),
                commit_oid: commit_oid.to_string(),
                ref_name: ref_name.to_string(),
                trigger_event: event_type.to_string(),
                actor_id: actor_id.to_string(),
                status: "QUEUED".to_string(),
                conclusion: None,
                definition_yaml: def.yaml_content.clone(),
                definition_digest: def.digest.clone(),
                created_at_nanos: now_nanos,
                started_at_nanos: None,
                completed_at_nanos: None,
            };

            let created_run = self.db.create_workflow_run(run).await?;

            // Create initial PR Check result in PENDING status
            let check_result = WorkflowCheckResult {
                id: Uuid::new_v4().to_string(),
                repo_id: repo_id.to_string(),
                commit_oid: commit_oid.to_string(),
                check_name: spec.name.clone(),
                status: "PENDING".to_string(),
                run_id: run_id.clone(),
                updated_at_nanos: now_nanos,
            };
            let _ = self.db.upsert_workflow_check_result(check_result).await;

            // Create jobs and steps
            for (job_key, job_spec) in &spec.jobs {
                let job_id = Uuid::new_v4().to_string();
                let initial_status = if job_spec.needs.is_empty() {
                    "QUEUED"
                } else {
                    "WAITING_FOR_RUNNER"
                };

                let runs_on = if job_spec.runs_on.is_empty() {
                    vec!["self-hosted".to_string()]
                } else {
                    job_spec.runs_on.clone()
                };

                let job = WorkflowJob {
                    id: job_id.clone(),
                    run_id: run_id.clone(),
                    repo_id: repo_id.to_string(),
                    job_key: job_key.clone(),
                    name: job_spec.name.clone().unwrap_or_else(|| job_key.clone()),
                    status: initial_status.to_string(),
                    runner_id: None,
                    runs_on,
                    needs: job_spec.needs.clone(),
                    attempt: 1,
                    fencing_token: 0,
                    lease_expires_at_nanos: None,
                    exit_code: None,
                    created_at_nanos: now_nanos,
                    started_at_nanos: None,
                    completed_at_nanos: None,
                };

                self.db.create_workflow_job(job).await?;

                // Create steps
                for (idx, step_spec) in job_spec.steps.iter().enumerate() {
                    let step = WorkflowStep {
                        id: Uuid::new_v4().to_string(),
                        job_id: job_id.clone(),
                        step_index: idx as i32,
                        name: step_spec
                            .name
                            .clone()
                            .unwrap_or_else(|| format!("Step {}", idx + 1)),
                        status: "QUEUED".to_string(),
                        exit_code: None,
                        started_at_nanos: None,
                        completed_at_nanos: None,
                    };
                    self.db.create_workflow_step(step).await?;
                }
            }

            // Dispatch webhook for workflow_run.queued
            if let Some(wh) = &self.webhook_service {
                let payload = serde_json::json!({
                    "action": "queued",
                    "workflow_run": created_run,
                    "repository_id": repo_id,
                });
                let _ = wh
                    .dispatch_event("REPO", repo_id, "workflow_run", &payload)
                    .await;
            }

            triggered_runs.push(created_run);
        }

        Ok(triggered_runs)
    }

    pub async fn get_run(&self, run_id: &str) -> Result<Option<WorkflowRun>, ForgeError> {
        self.db.get_workflow_run(run_id).await
    }

    pub async fn list_runs(
        &self,
        repo_id: &str,
        limit: i64,
    ) -> Result<Vec<WorkflowRun>, ForgeError> {
        self.db.list_workflow_runs(repo_id, limit).await
    }

    pub async fn get_job(&self, job_id: &str) -> Result<Option<WorkflowJob>, ForgeError> {
        self.db.get_workflow_job(job_id).await
    }

    pub async fn list_jobs(&self, run_id: &str) -> Result<Vec<WorkflowJob>, ForgeError> {
        self.db.list_workflow_jobs(run_id).await
    }

    pub async fn list_steps(&self, job_id: &str) -> Result<Vec<WorkflowStep>, ForgeError> {
        self.db.list_workflow_steps(job_id).await
    }

    pub async fn list_logs(
        &self,
        job_id: &str,
    ) -> Result<Vec<crate::core::types::WorkflowLogChunk>, ForgeError> {
        self.db.list_workflow_logs(job_id).await
    }

    pub async fn cancel_run(&self, run_id: &str) -> Result<(), ForgeError> {
        let now_nanos = Utc::now().timestamp_nanos_opt().unwrap_or(0);
        let jobs = self.db.list_workflow_jobs(run_id).await?;
        for job in jobs {
            if job.status == "QUEUED"
                || job.status == "WAITING_FOR_RUNNER"
                || job.status == "ASSIGNED"
                || job.status == "RUNNING"
            {
                self.db
                    .update_workflow_job_status(
                        &job.id,
                        "CANCELLED",
                        Some(130),
                        None,
                        Some(now_nanos),
                    )
                    .await?;
            }
        }
        self.db
            .update_workflow_run_status(
                run_id,
                "COMPLETED",
                Some("CANCELLED"),
                None,
                Some(now_nanos),
            )
            .await?;
        Ok(())
    }
}
